#!/usr/bin/env python3
"""Export the OneNote spec notebook to staging: one cleaned HTML file per page.

The OneNote web canvas is real HTML — tables are `<table>`/`<td>`, screenshots are
inline `data:` URIs — so this walks the section/page rails, lifts
`#PageContentContainer` per page, strips the editor chrome and writes:

    staging/<Section path>/<Page>.html     content, wrapped in <main> for any2md
    staging/<Section path>/<Page>.json     {title, status, section_path, images, src_sha}
    staging/<Section path>/images/<slug>/img_<md5[:12]>.png

    python export.py                     # everything
    python export.py --limit 3           # first 3 pages, for a smoke test
    python export.py --section Client    # one section only

Exit codes: 0 ok · 2 session dead (a human must log in) · 3 nothing found ·
8 not configured (SHAREPOINT_SITE, NOTEBOOK_URL, NOTEBOOK_PATH).
"""
import argparse
import base64
import hashlib
import json
import os
import re
import shutil
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

from pause import blocks, read_pause

ROOT = Path(__file__).parent
DATA = ROOT / 'data'
STATE = DATA / 'storage-state.json'
STAGING = DATA / 'staging'
PAUSED = DATA / 'PAUSED'                  # sentinel: a human must clear it in the admin,
                                          # except for a lockout, which Microsoft clears
ATTEMPTS = DATA / 'login-attempts.json'   # login budget, keyed by account email
ACCOUNT = DATA / 'ms-account.json'        # which account to sign in as, set in the admin
PENDING = DATA / 'pending.json'           # runner -> admin: "I need the emailed code"
OTP_ANSWER = DATA / 'otp.json'            # admin -> runner: the code the human typed
LOGIN_RESULT = DATA / 'login-result.json' # runner -> admin: why a sign-in did not work
SECTION_STATE = DATA / 'section-state.json'  # crawl cache {path: {len, mtime}}; losing it
                                             # only costs one full crawl

# which notebook — all from the environment (see .env.example), checked in main()
SITE = os.environ.get('SHAREPOINT_SITE', '').rstrip('/')       # https://<tenant>.sharepoint.com/sites/<site>
NB_FOLDER = os.environ.get('NOTEBOOK_PATH', '').rstrip('/')    # server-relative folder with the .one files
NOTEBOOK_URL = os.environ.get('NOTEBOOK_URL', '')               # the notebook as the browser opens it

# vault naming rules (same as the PDF pipeline, so paths stay stable)
BAD_NAME = re.compile(r'[:/><|?*\\]')
SLUG = re.compile(r'[^\w.\-]', re.UNICODE)

# a page title carries its workflow status; keep it out of the filename or every
# status change renames a file. "(drawer)", "(admin)" are part of the real name.
STATUS_WORDS = re.compile(
    r'^(to review|to validate|to confirm|to complete|in progress|done|validated|reviewed|'
    r'wip|en cours|à valider|a valider|à revoir|review)\b', re.I)
STATUS_TAIL = re.compile(r'\s*\(([^()]{1,60})\)\s*$')

DATE_LINE = re.compile(r'(?:lundi|mardi|mercredi|jeudi|vendredi|samedi|dimanche|Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)\s+\d{1,2}\s+\S+\s+\d{4}(?:\s+\d{1,2}[:h]\d{2})?', re.I)

MIN_IMG_BYTES = 3000        # below this it is a spacer or an icon, not a screenshot
CONTENT_SELECTOR = '#PageContentContainer'


def clean_name(raw):
    """Page title -> (file name, status). Mirrors the vault's existing names."""
    status = None
    name = ' '.join(raw.replace('\xa0', ' ').split())
    m = STATUS_TAIL.search(name)
    if m and STATUS_WORDS.match(m.group(1).strip()):
        status = m.group(1).strip()
        name = name[:m.start()].rstrip()
    return ' '.join(BAD_NAME.sub('-', name).split()), status


def slugify(name):
    return SLUG.sub('_', name).strip('_')


def onenote_frame(page):
    for f in page.frames:
        if 'onenoteframe' in f.url:
            return f
    return None


# Text settles before screenshots do: images arrive as data: URIs seconds later,
# and a screenshot that has not rendered yet is a `WACImage` with an EMPTY src —
# OneNote only fills it when the element enters the viewport. Below-the-fold
# screenshots therefore never load by themselves in headless, so each poll also
# nudges the first pending one into view to trigger its lazy render. A page is
# ready only when length AND the inlined-image count are stable for 3 polls AND
# nothing content-shaped is still pending.
MEASURE_JS = """
() => {
  const box = document.querySelector('#PageContentContainer');
  if (!box) return null;
  let data = 0;
  const pending = [];
  for (const n of box.querySelectorAll('img')) {
    const src = n.getAttribute('src') || '';
    if (src.startsWith('data:')) { data += 1; continue; }
    if (/(^|\\s)one_/.test(n.className)) continue;             // editor chrome
    if (src.includes('onecdn.static.microsoft')) continue;      // CDN chrome
    pending.push(n);
  }
  if (pending.length) pending[0].scrollIntoView({block: 'center'});
  return {size: box.innerHTML.length, data, pending: pending.length};
}
"""


def wait_for_content(frame, timeout_ms=90_000):
    """Content is async; wait until it stops growing and images are inlined."""
    # 'attached', not 'visible': headless renders the canvas into a container that
    # Playwright considers hidden, while its innerHTML is fully populated
    frame.wait_for_selector(CONTENT_SELECTOR, state='attached', timeout=timeout_ms)
    last, stable = None, 0
    for _ in range(timeout_ms // 500):
        m = frame.evaluate(MEASURE_JS)
        if m and m['size'] > 0 and m['pending'] == 0 and (m['size'], m['data']) == last:
            stable += 1
            if stable >= 3:
                return m['size']
        else:
            stable = 0
        last = (m['size'], m['data']) if m else None
        frame.wait_for_timeout(500)
    print('    ! content did not settle (images may be missing)')
    return last[0] if last else -1


# JS runs in the page: strip editor chrome, then hand back title + content HTML.
EXTRACT_JS = """
() => {
  const box = document.querySelector('#PageContentContainer');
  if (!box) return null;
  const clone = box.cloneNode(true);
  const junk = [
    'img.one_OutlineElementHandle_16x16x32', 'img[class^="one_Resize"]',
    'img[class^="one_"]', '.OutlineTitleArea', '.OutlineResizeHandleContainer',
    '.OutlineResizeHandle', '.WACAltTextDescribedBy', '.insertionHint',
    '[role="presentation"][aria-hidden="true"]', 'script', 'style',
  ];
  junk.forEach(sel => clone.querySelectorAll(sel).forEach(n => n.remove()));
  // editor icons served from the Office CDN are chrome, never spec content
  clone.querySelectorAll('img[src]').forEach(n => {
    if (!n.getAttribute('src').startsWith('data:')) n.remove();
  });
  // html5ever foster-parents OneNote's div-inside-td chains out of the table,
  // emptying every cell in the converter — so rebuild each table as a minimal
  // <tr><td> right here, where the live DOM is unambiguous. Cell text keeps
  // its line structure: block children join with <br>; nested tables flatten.
  const esc = s => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  const BLOCK = /^(DIV|P|LI|UL|OL|TABLE|TR|H[1-6])$/;
  const cellHtml = td => {
    let out = '';
    const brk = () => { if (out && !out.endsWith('<br>')) out += '<br>'; };
    const walk = n => {
      for (const c of n.childNodes) {
        if (c.nodeType === 3) { out += esc(c.textContent); continue; }
        if (c.nodeType !== 1) continue;
        if (c.tagName === 'BR') { out += '<br>'; continue; }
        if (c.tagName === 'IMG') {
          const src = c.getAttribute('src') || '';
          if (src.startsWith('data:')) out += `<img src="${src}">`;
          continue;
        }
        const block = BLOCK.test(c.tagName);
        if (block) brk();
        walk(c);
        if (block) brk();
      }
    };
    walk(td);
    return out.replace(/^(<br>)+/, '').replace(/(<br>)+$/, '');
  };
  [...clone.querySelectorAll('table')]
    .filter(t => !t.parentElement.closest('table'))
    .forEach(t => {
      const min = document.createElement('table');
      min.innerHTML = [...t.rows].map(r =>
        '<tr>' + [...r.cells].map(c => {
          const tag = c.tagName.toLowerCase();
          return `<${tag}>${cellHtml(c)}</${tag}>`;
        }).join('') + '</tr>').join('\\n');
      t.replaceWith(min);
    });
  const titleEl = clone.querySelector('.Title');
  const title = (titleEl?.innerText || document.title || '').trim();
  if (titleEl) titleEl.remove();
  return {title, html: clone.innerHTML};
}
"""


def save_images(html, outdir, slug):
    """Inline data: images -> files named by content hash; rewrite the refs.

    Content-addressed on purpose: a sequential counter renumbers every image after
    an insertion, and git then sees N new blobs for one real change.
    """
    seen, refs = {}, []
    imgdir = outdir / 'images' / slug

    def repl(m):
        mime, b64 = m.group(1), m.group(2)
        try:
            data = base64.b64decode(b64)
        except Exception:
            return m.group(0)
        if len(data) < MIN_IMG_BYTES:
            return ''                                  # spacer gif / tiny icon
        digest = hashlib.md5(data).hexdigest()[:12]
        ext = {'image/png': 'png', 'image/jpeg': 'jpg', 'image/gif': 'gif',
               'image/svg+xml': 'svg'}.get(mime, 'png')
        rel = f'images/{slug}/img_{digest}.{ext}'
        if digest not in seen:
            imgdir.mkdir(parents=True, exist_ok=True)
            (outdir / rel).write_bytes(data)
            seen[digest] = rel
            refs.append(rel)
        return f'src="{seen[digest]}"'

    html = re.sub(r'src="data:([^;]+);base64,([^"]*)"', repl, html)
    return html, refs


TREE_JS = """
() => {
  const lbl = e => (e.getAttribute('title')
                    || e.querySelector('[title]')?.getAttribute('title')
                    || e.querySelector('content')?.textContent
                    || e.innerText || '').replace(/\\s+/g, ' ').trim();
  const isGroup = e => e.hasAttribute('aria-expanded') || /sectionGroup/.test(e.className);
  return [...document.querySelectorAll('[role="treeitem"]')].map(e => {
    // a subsection lives in the container that follows its group's item
    let parent = null, node = e;
    while (node && node !== document.body) {
      const prev = node.previousElementSibling;
      if (prev && prev.getAttribute('role') === 'treeitem' && prev.hasAttribute('aria-expanded')) {
        parent = lbl(prev); break;
      }
      node = node.parentElement;
    }
    return {label: lbl(e), group: isGroup(e), parent};
  }).filter(x => x.label);
}
"""


def rest_sections(ctx):
    """Authoritative section map from SharePoint REST: {'Group/Section': {len, mtime}}.

    A section is a .one file in the notebook folder; a section group is a
    subfolder holding its own .one files. The rails can silently hide a whole
    collapsed group (it happened with Admin), the file listing cannot. Length and
    TimeLastModified double as the change signal: measured twice on 2026-07-31,
    our read-only visits move neither, a real edit moves both.
    """
    hdr = {'Accept': 'application/json;odata=nometadata'}

    def ls(folder, what):
        r = ctx.request.get(
            f"{SITE}/_api/web/GetFolderByServerRelativeUrl('{folder}')/{what}"
            '?$select=Name,Length,TimeLastModified', headers=hdr)
        if r.status != 200:
            raise RuntimeError(f'{what} listing → HTTP {r.status}')
        return r.json()['value']

    out = {}

    def norm(name):
        return ' '.join(name.replace('\xa0', ' ').split())   # match the rails' labels

    def add(files, prefix=''):
        for f in files:
            if f['Name'].endswith('.one'):
                out[prefix + norm(f['Name'][:-4])] = {'len': str(f['Length']),
                                                      'mtime': f['TimeLastModified']}

    add(ls(NB_FOLDER, 'Files'))
    for fold in ls(NB_FOLDER, 'Folders'):
        if fold['Name'] != 'OneNote_RecycleBin':
            add(ls(f"{NB_FOLDER}/{fold['Name']}", 'Files'), prefix=f"{norm(fold['Name'])}/")
    return out


def prune_section(sdir, keep_titles):
    """Drop staged pages (and their image dirs) the section no longer has."""
    slugs = {slugify(t) for t in keep_titles}
    for f in list(sdir.glob('*.html')) + list(sdir.glob('*.json')):
        if f.stem not in keep_titles:
            f.unlink()
    imgroot = sdir / 'images'
    if imgroot.is_dir():
        for d in imgroot.iterdir():
            if d.is_dir() and d.name not in slugs:
                shutil.rmtree(d)


def staged_section_dirs():
    """Relative paths of staging dirs that directly hold page files."""
    return sorted({j.parent.relative_to(STAGING).as_posix()
                   for j in STAGING.rglob('*.json')
                   if j.parent != STAGING and j.parent.name != 'images'})


def expand_groups(frame):
    """Section groups must be open to see their sections; OneNote remembers the
    state between visits, so this has to be idempotent rather than a toggle."""
    frame.wait_for_selector('[role="treeitem"]', state='attached', timeout=15_000)
    for _ in range(10):
        collapsed = frame.locator('[role="treeitem"][aria-expanded="false"]')
        if collapsed.count() == 0:
            return
        try:
            collapsed.first.click(timeout=6000, force=True)
        except Exception:
            return
        frame.page.wait_for_timeout(1200)


def list_sections(frame, expected=None):
    """Flat section list with the owning group, so paths mirror the vault tree.

    The rails hydrate late and a collapsed group hides its sections entirely
    (runs saw 11 sections where SharePoint has 17) — so when the REST listing
    says which paths must exist, keep expanding until they all show up or the
    grace budget runs out.
    """
    deadline = time.time() + 45
    while True:
        try:
            expand_groups(frame)
        except Exception as e:
            print(f'    ! rails: {str(e).splitlines()[0]}')
        out = []
        for it in frame.evaluate(TREE_JS):
            if it['group']:
                continue                 # groups are folders, not clickable sections
            path = f"{it['parent']}/{it['label']}" if it['parent'] else it['label']
            if path not in [o['path'] for o in out]:
                out.append({'label': it['label'], 'path': path})
        if not expected or expected <= {o['path'] for o in out} or time.time() > deadline:
            return out
        frame.page.wait_for_timeout(1500)


def list_pages(frame):
    return [t for t in frame.eval_on_selector_all(
        '#PageList [title]',
        "els => els.map(e => (e.getAttribute('title') || '').trim())") if t]


# Playwright treats the rails as hidden in headless, so clicks go through the DOM.
# Whitespace is normalised on both sides: the rails are full of NBSPs.
CLICK_JS = """
([scope, label]) => {
  const norm = s => (s || '').replace(/\\u00a0/g, ' ').replace(/\\s+/g, ' ').trim();
  const want = norm(label);
  // match the title attribute only: a section group carries the same visible text
  // as the section nested inside it, and clicking the group just collapses it
  const cands = [...document.querySelectorAll(scope + ' [title]')];
  const hit = cands.find(e => norm(e.getAttribute('title')) === want);
  if (!hit) return false;
  (hit.closest('[role="treeitem"]') || hit).click();
  return true;
}
"""


def click_nav(frame, label, what, scope):
    try:
        if frame.evaluate(CLICK_JS, [scope, label]):
            frame.page.wait_for_timeout(2500)
            return True
    except Exception as e:
        print(f'    ! click error on {what} {label!r}: {str(e).splitlines()[0]}')
        return False
    print(f'    ! cannot open {what} {label!r}')
    return False


def export_page(frame, section_path, raw_title):
    name, status = clean_name(raw_title)
    slug = slugify(name)
    outdir = STAGING / section_path
    outdir.mkdir(parents=True, exist_ok=True)

    wait_for_content(frame)
    got = frame.evaluate(EXTRACT_JS)
    if got is None:
        print(f'    ! no content container for {name!r}')
        return None

    # a genuinely empty page (title only) is still a page: write the title-only
    # note, or the run counts as partial and deletions stay suppressed forever
    html, refs = save_images(got['html'], outdir, slug)
    # the canvas title element repeats the title (a11y copy) and appends the page
    # date, so the heading comes from the clean name and the date is lifted out
    date = DATE_LINE.search(got.get('title') or '')
    head = f'<h1>{name}</h1>\n' + (f'<p><em>{date.group(0)}</em></p>\n' if date else '')
    doc = f'<!-- {name} -->\n<main>\n{head}{html}\n</main>\n'
    (outdir / f'{name}.html').write_text(doc)

    meta = {'title': name, 'raw_title': raw_title, 'status': status,
            'section_path': section_path, 'images': refs,
            'src_sha': hashlib.sha256(doc.encode()).hexdigest(),
            'tables': doc.count('<table'), 'bytes': len(doc)}
    (outdir / f'{name}.json').write_text(json.dumps(meta, indent=2, ensure_ascii=False))
    print(f'    ok {section_path}/{name}.html  '
          f'({len(doc)//1024}KB, tables={meta["tables"]}, imgs={len(refs)}'
          f'{", status=" + status if status else ""})')
    return meta


AADSTS = re.compile(r'AADSTS(\d{5,6})')
AADSTS_TEXT = {'50126': 'wrong password', '50053': 'account locked (smart lockout)',
               '50055': 'password expired', '50076': 'MFA required', '50079': 'MFA required',
               '53003': 'blocked by Conditional Access', '50196': 'client loop / throttled'}


LOGIN_BUDGET = 5    # sign-in attempts per account per 24 h
MAX_CODES = 3       # rejected codes one attempt absorbs before it gives up
CODE_GRACE = 30     # seconds a submitted code gets to round-trip before the code
                    # screen still being there counts as a rejection. Generous on
                    # purpose: calling an accepted code rejected throws a working
                    # session away, where being slow to offer a retry only annoys.
SCREEN_MOVED_ON = object()   # the screen answered while we waited for a human


def account_email():
    """The account to sign in as: whatever the admin last saved, else the env var.

    The saved file wins so switching account needs no redeploy, and the env var
    keeps existing deployments working with nothing set.

    Lower-cased because this string is the key the daily allowance is counted
    under, while Microsoft treats the address case-insensitively: without it the
    same mailbox typed with a capital would be handed a second allowance, and the
    brake that stops a runaway loop locking the account out counts the wrong
    bucket.
    """
    try:
        saved = json.loads(ACCOUNT.read_text()).get('email')
    except (OSError, ValueError, AttributeError):
        saved = None
    return ((saved if isinstance(saved, str) else None)
            or os.environ.get('MS_EMAIL') or '').strip().lower()


def _login_budget_ok(email):
    """Max 5 login attempts per 24h per account — a runaway loop must not lock an
    account out. The allowance is per account, so a freshly entered address is not
    refused for attempts the previous one spent.

    The key is folded here rather than trusted from the caller: this is the only
    thing standing between a runaway loop and a locked-out account, and Microsoft
    does not read the capitals the way a dict key does.
    """
    email = email.strip().lower()
    now = time.time()
    try:
        raw = json.loads(ATTEMPTS.read_text())
    except (OSError, ValueError):
        # ponytail: an unreadable log grants the allowance rather than withholding
        # it, so a hand-edited file can never lock a human out of their own admin.
        # The ceiling is that a torn log resets the brake, which is why the write
        # below is atomic — replace this with a real store if the log ever needs to
        # survive something worse than a text editor.
        raw = {}
    # a flat list is from before the log was keyed by account: it belongs to
    # whoever was signing in then, so it counts here until it ages out
    if isinstance(raw, list):
        raw = {email: raw}
    spent = {e: [t for t in ts if isinstance(t, (int, float)) and now - t < 86_400]
             for e, ts in (raw.items() if isinstance(raw, dict) else [])
             if isinstance(ts, list)}
    if len(spent.get(email, [])) >= LOGIN_BUDGET:
        print(f'login budget exhausted ({LOGIN_BUDGET} per 24h for {email}) — not trying',
              file=sys.stderr)
        return False
    spent[email] = spent.get(email, []) + [now]
    # through a temp file: a write torn by a redeploy or a full disk would read
    # back as no log at all, and the one thing this brake must not do is fail open
    # for the runaway loop it exists to stop
    tmp = ATTEMPTS.with_suffix('.tmp')
    tmp.write_text(json.dumps({e: ts for e, ts in spent.items() if ts}))
    tmp.replace(ATTEMPTS)
    return True


def _login_failed(reason):
    """Fail, and leave the reason where the admin can read it: the session card
    shows what went wrong, and 'see the log' is not something a human can act on.
    """
    print(f'  login failed: {reason}', file=sys.stderr)
    LOGIN_RESULT.write_text(json.dumps({'reason': reason}))
    return False


def _await_code(hint, deadline, rejected=False, moved_on=None):
    """Publish pending.json and wait for the admin to submit the emailed code.

    `deadline` is the sign-in's own: a second prompt after a rejected code shares
    the window with the first rather than waiting past the point the runner quits.

    `moved_on` says the screen we are collecting a code for is gone — the last one
    was accepted after all, just slowly. Waiting on for a code nobody needs would
    end in a timeout that discards a session which actually works, so the wait
    ends there and the caller goes back to reading the page.
    """
    deadline = min(deadline, time.time() + 600)
    PENDING.write_text(json.dumps({
        'kind': 'otp', 'hint': hint, 'rejected': rejected,
        'since': datetime.now(timezone.utc).isoformat(timespec='seconds'),
        'deadline_epoch': deadline}))
    OTP_ANSWER.unlink(missing_ok=True)
    try:
        while time.time() < deadline:
            if moved_on is not None and moved_on():
                return SCREEN_MOVED_ON
            if OTP_ANSWER.exists():
                code = json.loads(OTP_ANSWER.read_text()).get('code', '').strip()
                OTP_ANSWER.unlink(missing_ok=True)
                if code:
                    return code
            time.sleep(2)
    finally:
        PENDING.unlink(missing_ok=True)
    return None


def try_login(page):
    """Drive the Microsoft sign-in as the saved account (+ optional M365_PASSWORD);
    the emailed code comes from the admin UI. True = notebook reachable again.

    State machine over what is on screen, not a fixed sequence: Microsoft
    reorders the steps. Errors classify by AADSTS code, never by DOM copy.
    50053 (lockout) writes the PAUSED sentinel, and that flavour of pause stops
    everything including further sign-ins — retrying only deepens the lockout.
    """
    LOGIN_RESULT.unlink(missing_ok=True)
    email = account_email()
    if not email:
        return _login_failed('no account email set — enter one in the admin, or prime '
                             'the session by hand with recon.py --fresh')
    if not _login_budget_ok(email):
        return _login_failed(f'no sign-in attempts left today for {email} '
                             f'({LOGIN_BUDGET} per 24 h)')

    # the emailed one-time code input: classic AAD uses name=otc, this tenant's
    # guest flow uses name=npotc — the id covers both page generations
    OTP_SEL = 'input[name="otc"], input[name="npotc"], #idTxtBx_OTC_Password'

    filled_email = filled_pw = warned_pw = False
    codes_tried, code_at = 0, None
    deadline = time.time() + 720
    while time.time() < deadline:
        if 'sharepoint.com' in urlparse(page.url).netloc:
            print('  login ok')
            LOGIN_RESULT.unlink(missing_ok=True)
            return True
        body = page.content()
        m = AADSTS.search(body)
        if m:
            code = m.group(1)
            if code == '50053':
                PAUSED.write_text(json.dumps({'reason': f'AADSTS{code} smart lockout',
                                              'at': datetime.now(timezone.utc).isoformat()}))
            return _login_failed(f'AADSTS{code}'
                                 + (f' — {AADSTS_TEXT[code]}' if code in AADSTS_TEXT else ''))

        def visible(sel):
            loc = page.locator(sel).first
            try:
                return loc.is_visible()
            except Exception:
                return False

        # the code page wins over the password branch: the first page shows the
        # email and password inputs together, and this tenant's flow jumps from
        # there straight to "enter the code we just emailed you"
        if not filled_email and visible('input[type="email"]'):
            page.fill('input[type="email"]', email)
            page.keyboard.press('Enter')
            filled_email = True
        # A rejected code re-renders the same code screen, so the screen alone
        # cannot say whether the last code is still being checked — but a code
        # screen still standing CODE_GRACE seconds after a code went in was a
        # rejection, and the human gets to type another one. Remembering only
        # "a code was filled once" made a mistyped digit indistinguishable from a
        # hang: nothing could prompt again, the loop spun to the deadline, and the
        # attempt was gone from the five the account gets per day.
        elif visible(OTP_SEL) and (code_at is None or time.time() - code_at > CODE_GRACE):
            if codes_tried >= MAX_CODES:
                return _login_failed(f'{MAX_CODES} codes were rejected')
            code = _await_code(
                'that code was rejected — check the email and try again' if codes_tried
                else 'Microsoft asks for the code emailed to ' + email,
                deadline, rejected=bool(codes_tried),
                moved_on=lambda: not visible(OTP_SEL))
            if code is SCREEN_MOVED_ON:
                continue          # accepted after all — read the page again
            if not code:
                return _login_failed('no code was entered in time')
            page.fill(OTP_SEL, code)
            page.keyboard.press('Enter')
            codes_tried, code_at = codes_tried + 1, time.time()
        elif not filled_pw and filled_email and visible('input[type="password"]'):
            pw = os.environ.get('M365_PASSWORD')
            if pw:
                page.fill('input[type="password"]', pw)
                page.keyboard.press('Enter')
                filled_pw = True
            elif not warned_pw:
                # not fatal: the combined first page keeps the password input
                # visible for a moment after the email submit — keep polling
                print('  password prompt visible and M365_PASSWORD unset — waiting')
                warned_pw = True
        elif visible('#idSIButton9'):          # "Stay signed in?" — yes, keep the cookie
            page.click('#idSIButton9')
        page.wait_for_timeout(2000)
    return _login_failed('the sign-in did not finish in time')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--limit', type=int, help='stop after N pages')
    ap.add_argument('--section', help='only this section')
    ap.add_argument('--headed', action='store_true')
    ap.add_argument('--login-only', action='store_true',
                    help='just (re)establish the session and save storage state')
    ap.add_argument('--ignore-pause', action='store_true',
                    help='a human asked for this run despite the pause (a lockout still stops it)')
    args = ap.parse_args()

    # cleared here rather than in try_login, which the exits below never reach: a
    # reason left over from the last attempt would otherwise be served as the
    # reason for this one, and 'wrong password' reads very differently from the
    # lockout that actually stopped it
    LOGIN_RESULT.unlink(missing_ok=True)

    # signing in is what gets a paused install unstuck, so a human pause must
    # never stand in its way; a lockout pause blocks everything, always
    paused = read_pause(PAUSED)
    if blocks(paused, by_hand=args.login_only or args.ignore_pause):
        print(f'PAUSED: {paused.get("reason", "paused")} — clear it in the admin',
              file=sys.stderr)
        return 7
    if not STATE.exists() and not account_email():
        print('no session — run recon.py first to log in', file=sys.stderr)
        return 2
    missing = [name for name, value in (('SHAREPOINT_SITE', SITE), ('NOTEBOOK_URL', NOTEBOOK_URL),
                                        ('NOTEBOOK_PATH', NB_FOLDER)) if not value]
    if missing:
        print(f'not configured: set {", ".join(missing)} (see .env.example)', file=sys.stderr)
        return 8

    STAGING.mkdir(parents=True, exist_ok=True)
    exported, seen, done = [], [], 0

    with sync_playwright() as pw:
        browser = pw.chromium.launch(headless=not args.headed)
        ctx = browser.new_context(storage_state=str(STATE) if STATE.exists() else None,
                                  viewport={'width': 1600, 'height': 1000}, locale='fr-FR')
        page = ctx.new_page()
        page.goto(NOTEBOOK_URL, wait_until='domcontentloaded', timeout=120_000)
        page.wait_for_timeout(9000)

        if 'login.microsoftonline' in page.url or not STATE.exists():
            if not try_login(page):
                browser.close()
                return 2
            page.wait_for_timeout(5000)
            if 'sharepoint.com' not in urlparse(page.url).netloc:
                page.goto(NOTEBOOK_URL, wait_until='domcontentloaded', timeout=120_000)
                page.wait_for_timeout(9000)

        if args.login_only:
            ctx.storage_state(path=str(STATE))
            os.chmod(STATE, 0o600)
            browser.close()
            print('session saved')
            return 0

        frame = onenote_frame(page)
        if frame is None:
            print('no OneNote frame — is this the notebook URL?', file=sys.stderr)
            browser.close()
            return 3

        # SharePoint's file listing is the authority on which sections exist and
        # whether they changed; the rails only say what happens to be rendered
        rest = None
        try:
            rest = rest_sections(ctx)
            print(f'REST sections ({len(rest)}): {sorted(rest)}')
        except Exception as e:
            print(f'  ! REST listing failed ({e}) — full crawl, no skip, no prune')

        full_run = not args.section and not args.limit
        sec_state = json.loads(SECTION_STATE.read_text()) if SECTION_STATE.exists() else {}
        skip = set()
        if rest and full_run:
            skip = {p for p, i in rest.items()
                    if sec_state.get(p) == i and any((STAGING / p).glob('*.json'))}

        sections = list_sections(frame, expected=set(rest) if rest else None)
        print(f'rails sections ({len(sections)}): {[s["path"] for s in sections]}')
        partial = False
        if rest:
            rails = {s['path'] for s in sections}
            hidden = sorted(p for p in rest if p not in rails and p not in skip)
            if hidden:
                print(f'  ! in SharePoint but not reachable in the rails: {hidden}'
                      ' — partial run, deletions will be suppressed')
                partial = True

        new_state = dict(sec_state)
        for sec in sections:
            if args.section and args.section not in (sec['path'], sec['label']):
                continue
            if sec['path'] in skip:
                continue
            if args.limit and done >= args.limit:
                break
            print(f'  section {sec["path"]}')
            if not click_nav(frame, sec['label'], 'section', '[role="treeitem"]'):
                new_state.pop(sec['path'], None)
                partial = True
                continue
            pages = list_pages(frame)
            seen.extend(f'{sec["path"]}/{p}' for p in pages)
            print(f'    {len(pages)} pages')
            ok_titles = []
            for t in pages:
                if args.limit and done >= args.limit:
                    break
                if not click_nav(frame, t, 'page', '#PageList'):
                    continue
                meta = export_page(frame, sec['path'], t)
                done += 1
                if meta:
                    exported.append(meta)
                    ok_titles.append(meta['title'])
            section_complete = len(ok_titles) == len(pages)
            if not args.limit and section_complete:
                prune_section(STAGING / sec['path'], set(ok_titles))
            if rest and sec['path'] in rest:
                if section_complete:
                    new_state[sec['path']] = rest[sec['path']]
                else:
                    new_state.pop(sec['path'], None)   # re-crawl it next run

        # skipped sections stand in through their staged copies
        for path in sorted(skip):
            metas = [json.loads(j.read_text())
                     for j in sorted((STAGING / path).glob('*.json'))]
            print(f'  skip {path} — unchanged upstream ({len(metas)} staged pages)')
            seen.extend(f'{path}/{m["raw_title"]}' for m in metas)
            exported.extend(metas)

        # sections gone from SharePoint disappear from staging too
        if rest and full_run:
            for rel in staged_section_dirs():
                if rel not in rest:
                    print(f'  - section gone upstream: {rel}')
                    shutil.rmtree(STAGING / rel)
                    new_state.pop(rel, None)

        if rest:
            SECTION_STATE.write_text(json.dumps(new_state, indent=2, ensure_ascii=False,
                                                sort_keys=True))
        ctx.storage_state(path=str(STATE))       # keep the session warm
        os.chmod(STATE, 0o600)
        browser.close()

    # seen vs exported (plus the explicit flag) lets the sync tell a partial
    # run from real deletions
    (STAGING / 'index.json').write_text(json.dumps(
        {'seen': seen, 'exported': exported, 'partial': partial},
        indent=2, ensure_ascii=False))
    print(f'\n{len(exported)}/{len(seen)} pages staged in {STAGING}'
          + (' (partial)' if partial else ''))
    return 0 if exported else 3


if __name__ == '__main__':
    sys.exit(main())
