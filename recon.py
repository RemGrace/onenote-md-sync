#!/usr/bin/env python3
"""Recon spike: what can we actually get out of the OneNote notebook, and how.

Opens a HEADED browser, hands it to you for the login (including the code that
arrives by email), then captures everything needed to choose an extraction
method. Run it once, read `data/recon/REPORT.md`, then build the exporter.

    python recon.py                      # full run, uses a saved session if there is one
    python recon.py --fresh              # ignore the saved session, log in again

Everything lands in ./data/recon/ — which holds a live session and raw tokens,
so it is gitignored, and it is not something to paste into a chat.
"""
import argparse
import base64
import json
import os
import re
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import TimeoutError as PWTimeout
from playwright.sync_api import sync_playwright

NOTEBOOK_URL = os.environ.get('NOTEBOOK_URL', '')
SITE = os.environ.get('SHAREPOINT_SITE', '').rstrip('/')
# the folder that holds the notebook, one level above NOTEBOOK_PATH
SPEC_FOLDER = os.environ.get('NOTEBOOK_PATH', '').rstrip('/').rpartition('/')[0]

OUT = Path(__file__).parent / 'data' / 'recon'
STATE = Path(__file__).parent / 'data' / 'storage-state.json'
LOGIN_HOSTS = ('login.microsoftonline.com', 'login.live.com', 'login.microsoft.com')
SLUGISH = re.compile(r'[^\w.\-]', re.UNICODE)
LOGIN_TIMEOUT_S = 15 * 60          # you have to read an email; take your time


def jwt_claims(token):
    """Header+payload claims of a JWT, without ever keeping the token itself."""
    parts = token.split('.')
    if len(parts) < 2:
        return {'note': 'not a JWT'}
    out = {}
    for name, raw in (('hdr', parts[0]), ('payload', parts[1])):
        try:
            pad = '=' * (-len(raw) % 4)
            out[name] = json.loads(base64.urlsafe_b64decode(raw + pad))
        except Exception as e:
            out[name] = {'undecodable': str(e)}
    p = out.get('payload', {})
    return {k: p.get(k) for k in ('aud', 'iss', 'appid', 'app_displayname', 'scp',
                                  'roles', 'tid', 'upn', 'unique_name', 'exp')}


def looks_logged_in(page):
    host = urlparse(page.url).netloc
    return host.endswith('sharepoint.com') and not host.startswith(LOGIN_HOSTS)


def wait_for_login(page):
    """Park until the human finishes the interactive login."""
    print(f'\n  A browser window is open. Log in there (the code goes to your email).')
    print(f'  Waiting up to {LOGIN_TIMEOUT_S // 60} min. Ctrl-C to give up.\n')
    deadline = time.time() + LOGIN_TIMEOUT_S
    last = None
    while time.time() < deadline:
        host = urlparse(page.url).netloc
        if host != last:
            print(f'  … at {host}')
            last = host
        if looks_logged_in(page):
            page.wait_for_timeout(4000)          # let the OneNote app boot
            return True
        # surface Microsoft's own diagnosis rather than guessing from the DOM
        try:
            body = page.inner_text('body', timeout=1500)
            code = re.search(r'AADSTS\d{5,6}', body or '')
            if code:
                print(f'  Microsoft says {code.group(0)}')
        except Exception:
            pass
        page.wait_for_timeout(2000)
    return False


def capture_requests(context, sink):
    """Record every request's shape, and the CLAIMS of any bearer token."""
    def on_request(req):
        auth = req.headers.get('authorization', '')
        row = {'method': req.method, 'url': req.url.split('?')[0],
               'type': req.resource_type, 'query': bool('?' in req.url)}
        if auth.lower().startswith('bearer '):
            row['token_claims'] = jwt_claims(auth.split(' ', 1)[1])
        sink.append(row)
    context.on('request', on_request)


def frame_dumps(page):
    """Every frame's HTML — the OneNote canvas usually lives in a nested one."""
    dumps = []
    for i, fr in enumerate(page.frames):
        try:
            html = fr.content()
        except Exception as e:
            dumps.append({'url': fr.url, 'error': str(e)})
            continue
        tables = html.count('<table')
        dumps.append({'i': i, 'url': fr.url, 'bytes': len(html), 'tables': tables})
        (OUT / f'frame_{i}.html').write_text(html)
    return dumps


def rest_probes(context):
    """Can we talk to SharePoint's own API with just this session?"""
    probes = {
        'currentUser': f"{SITE}/_api/web/currentUser",
        'specFolderFiles': (f"{SITE}/_api/web/GetFolderByServerRelativeUrl("
                            f"'{SPEC_FOLDER}')/Files"),
        'specFolderFolders': (f"{SITE}/_api/web/GetFolderByServerRelativeUrl("
                              f"'{SPEC_FOLDER}')/Folders"),
    }
    out = {}
    for name, url in probes.items():
        try:
            r = context.request.get(url, headers={'Accept': 'application/json;odata=nometadata'})
            body = r.text()[:1500]
            out[name] = {'status': r.status, 'body_head': body}
        except Exception as e:
            out[name] = {'error': str(e)}
        print(f'  REST {name}: {out[name].get("status", out[name].get("error"))}')
    return out


def write_report(data):
    r = [f"# Recon report — {datetime.now(timezone.utc).isoformat(timespec='seconds')}",
         '', f"Notebook URL host: `{urlparse(NOTEBOOK_URL).netloc}`", '']

    r += ['## 1-2. Bearer tokens seen, by audience', '']
    auds = {}
    for row in data['requests']:
        c = row.get('token_claims')
        if not c:
            continue
        key = str(c.get('aud'))
        a = auds.setdefault(key, {'count': 0, 'scp': c.get('scp'), 'appid': c.get('appid'),
                                  'app': c.get('app_displayname'), 'tid': c.get('tid'),
                                  'sample_url': row['url']})
        a['count'] += 1
    if auds:
        for aud, a in sorted(auds.items(), key=lambda kv: -kv[1]['count']):
            r += [f"- `{aud}` ×{a['count']} — app `{a['app'] or a['appid']}`, tid `{a['tid']}`",
                  f"  - scp: `{a['scp']}`", f"  - e.g. {a['sample_url']}"]
    else:
        r += ['- none captured — the app talks over cookies only, so candidate (a) is out.']

    r += ['', '## 3. Frames / canvas HTML', '']
    for f in data['frames']:
        if 'error' in f:
            r.append(f"- frame {f.get('url')}: ERROR {f['error']}")
        else:
            r.append(f"- frame {f['i']} `{f['url'][:90]}` — {f['bytes']}B, "
                     f"**{f['tables']} `<table>`** → `frame_{f['i']}.html`")

    r += ['', '## 4. Session cookies', '']
    for c in data['cookies']:
        exp = c.get('expires', -1)
        when = ('session' if not exp or exp < 0
                else datetime.fromtimestamp(exp, timezone.utc).isoformat(timespec='minutes'))
        r.append(f"- `{c['name']}` @ `{c['domain']}` — expires {when}")

    r += ['', '## 5. SharePoint REST with this session', '']
    for name, res in data['rest'].items():
        r.append(f"### {name} → {res.get('status', res.get('error'))}")
        if res.get('body_head'):
            r += ['```json', res['body_head'], '```']

    r += ['', '## 6. Endpoint inventory (top hosts)', '']
    hosts = {}
    for row in data['requests']:
        h = urlparse(row['url']).netloc
        hosts[h] = hosts.get(h, 0) + 1
    for h, n in sorted(hosts.items(), key=lambda kv: -kv[1])[:15]:
        r.append(f'- {h} ×{n}')

    r += ['', '## Decision', '',
          '- (a) internal/Graph API with a harvested token → viable only if an audience above '
          'covers page content.',
          '- (b) canvas DOM → viable if a frame above shows real `<table>` elements.',
          '- (c) print-to-PDF → always available (see `page.pdf` artifact), worst tables.', '']
    (OUT / 'REPORT.md').write_text('\n'.join(r))


def open_spec_page(page, section, title):
    """Click through the OneNote rails to a real page, then wait for its content.

    The notebook opens on 'Introduction', which is empty — dumping that answers
    nothing about how tables are rendered.
    """
    fr = next((f for f in page.frames if 'onenoteframe' in f.url), None)
    if fr is None:
        print('  no onenote frame found'); return False
    for label in (section, title):
        try:
            fr.get_by_text(label, exact=True).first.click(timeout=30_000)
            print(f'  clicked {label!r}')
            page.wait_for_timeout(3500)
        except Exception as e:
            print(f'  could not click {label!r}: {str(e).splitlines()[0]}')
            return False
    page.wait_for_timeout(6000)              # let the canvas paint
    return True


def dump_page(page, tag):
    """Per-page artifacts: every frame's HTML, a PDF and a screenshot."""
    sub = OUT / tag
    sub.mkdir(parents=True, exist_ok=True)
    found = []
    for i, fr in enumerate(page.frames):
        try:
            html = fr.content()
        except Exception:
            continue
        (sub / f'frame_{i}.html').write_text(html)
        found.append({'i': i, 'url': fr.url[:90], 'bytes': len(html),
                      'tables': html.count('<table'), 'tds': html.count('<td'),
                      'imgs': html.count('<img')})
    for f in found:
        print(f"  frame {f['i']}: {f['bytes']}B  <table>×{f['tables']}  "
              f"<td>×{f['tds']}  <img>×{f['imgs']}")
    try:
        page.pdf(path=str(sub / 'page.pdf'), format='A3', landscape=True,
                 print_background=True)
    except Exception as e:
        print(f'  pdf failed: {e}')
    page.screenshot(path=str(sub / 'page.png'), full_page=True)
    (sub / 'frames.json').write_text(json.dumps(found, indent=2))
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--fresh', action='store_true', help='ignore the saved session')
    ap.add_argument('--open', nargs=2, metavar=('SECTION', 'PAGE'),
                    help='after login, open this page and dump it (table-heavy one)')
    ap.add_argument('--headed', action='store_true', help='show the browser')
    args = ap.parse_args()

    missing = [name for name, value in (('NOTEBOOK_URL', NOTEBOOK_URL), ('SHAREPOINT_SITE', SITE),
                                        ('NOTEBOOK_PATH', SPEC_FOLDER)) if not value]
    if missing:
        print(f'not configured: set {", ".join(missing)} (see .env.example)', file=sys.stderr)
        return 1

    OUT.mkdir(parents=True, exist_ok=True)
    reuse = STATE.exists() and not args.fresh
    print(f'session: {"reusing " + str(STATE) if reuse else "fresh login"}')

    # a fresh login needs a human at the keyboard; reusing a session does not
    headed = args.headed or not reuse
    with sync_playwright() as pw:
        browser = pw.chromium.launch(headless=not headed, slow_mo=60 if headed else 0)
        context = browser.new_context(
            storage_state=str(STATE) if reuse else None,
            viewport={'width': 1600, 'height': 1000},
            locale='fr-FR',
            record_har_path=str(OUT / 'session.har'),
            record_har_content='embed',
        )
        requests = []
        capture_requests(context, requests)
        page = context.new_page()

        print(f'opening notebook…')
        page.goto(NOTEBOOK_URL, wait_until='domcontentloaded', timeout=120_000)

        if not looks_logged_in(page) and not wait_for_login(page):
            print('login did not complete — nothing captured beyond the HAR', file=sys.stderr)
            context.close(); browser.close()
            return 1

        print('logged in; letting the app settle…')
        page.wait_for_timeout(8000)

        STATE.parent.mkdir(parents=True, exist_ok=True)
        context.storage_state(path=str(STATE))
        os.chmod(STATE, 0o600)

        if args.open:
            section, title = args.open
            print(f'opening {section} / {title} …')
            if open_spec_page(page, section, title):
                tag = SLUGISH.sub('_', f'{section}__{title}')
                dump_page(page, tag)
                print(f'  artifacts in {OUT / tag}')

        data = {'requests': requests,
                'cookies': context.cookies(),
                'frames': frame_dumps(page),
                'rest': rest_probes(context)}

        try:                                       # candidate (c), the floor
            page.pdf(path=str(OUT / 'page.pdf'), format='A3', landscape=True,
                     print_background=True)
            print('  page.pdf written')
        except Exception as e:
            print(f'  page.pdf failed: {e}')

        page.screenshot(path=str(OUT / 'page.png'), full_page=True)
        (OUT / 'requests.json').write_text(json.dumps(requests, indent=2, ensure_ascii=False))
        write_report(data)

        context.close()
        browser.close()

    for f in OUT.glob('session.har'):
        os.chmod(f, 0o600)
    print(f'\nreport: {OUT / "REPORT.md"}')
    print('NOTE: data/recon/ holds a live session, raw tokens and the HAR. Do not share it.')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except PWTimeout as e:
        print(f'timeout: {e}', file=sys.stderr)
        sys.exit(2)
    except KeyboardInterrupt:
        print('\naborted', file=sys.stderr)
        sys.exit(130)
