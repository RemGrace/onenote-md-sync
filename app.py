#!/usr/bin/env python3
"""Admin for the spec sync: a sign-in page, a static bundle, one JSON, a handful
of POSTs.

    ADMIN_TOKEN=... ./.venv/bin/python app.py      # http://127.0.0.1:8377/

Shows whether the sync is healthy (age of the last success — the one number that
makes silent breakage visible), probes the session without a browser, runs a sync
on demand, accepts the emailed code when a login needs it, and pauses/resumes.

The UI itself is the built React bundle in frontend/dist; this module renders
only the token sign-in form, because that form is what decides whether the
bundle is handed over at all. Everything a human reads is already worded in
/status.json, so the browser lays facts out and never re-derives them.

The scheduler is a thread: "if we have not run today and it is past the
configured time, run". The time and the zone it is read in are a saved setting
(RUN_AT and the container's clock are the defaults). One attempt per morning;
retries are a human clicking Run now.
"""
import hmac
import json
import os
import re
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from zoneinfo import ZoneInfo

from flask import Flask, Response, redirect, request, send_from_directory

from pause import is_lockout, read_pause

ROOT = Path(__file__).parent
DATA = ROOT / 'data'
TOKEN = os.environ.get('ADMIN_TOKEN')
RUN_AT = os.environ.get('RUN_AT', '08:30')  # HH:MM default, until a setting is saved
SITE = os.environ.get('SHAREPOINT_SITE', '').rstrip('/')


def vault_web_from_repo(repo):
    """git@github.com:you/notes.git or https://github.com/you/notes.git -> https://github.com/you/notes"""
    m = re.match(r'^(?:git@([^:]+):|https://([^/]+)/)(.+?)(?:\.git)?/?$', repo)
    return f'https://{m[1] or m[2]}/{m[3]}' if m else ''


VAULT_WEB = (os.environ.get('VAULT_WEB', '').rstrip('/')
             or vault_web_from_repo(os.environ.get('VAULT_REPO', '')))

PAUSED = DATA / 'PAUSED'
PENDING = DATA / 'pending.json'
OTP_ANSWER = DATA / 'otp.json'
STATE = DATA / 'storage-state.json'
RUNS = DATA / 'runs.jsonl'
LAST_RUN = DATA / 'last_run.json'
RUN_LOGS = DATA / 'runs'         # one log per run, named by that run's id
SCHEDULE = DATA / 'schedule.json'
SCHEDULE_SETTINGS = DATA / 'schedule-settings.json'
ATTEMPTS = DATA / 'login-attempts.json'
ACCOUNT = DATA / 'ms-account.json'
LOGIN_RESULT = DATA / 'login-result.json'

LOGIN_BUDGET = 5           # sign-in attempts per account per 24 h, enforced in export.py

DIST = ROOT / 'frontend' / 'dist'
SPA_PAGES = ('runs', 'settings')     # every path the client-side router owns

# The bundle is registered as this app's static folder on purpose: routes on the
# app go through before_request, so the hashed assets sit behind the sign-in gate
# like everything else. A blueprint or a bare send_file route mounted outside it
# would hand the whole UI to anonymous visitors.
app = Flask(__name__, static_folder=DIST / 'assets', static_url_path='/assets')
app.config['MAX_CONTENT_LENGTH'] = 16 * 1024 * 1024   # absurd bodies die in werkzeug
proc_lock = threading.Lock()
proc = None          # the running sync/login subprocess, if any

# Where the sign-in this process started got to. A running runner's progress is
# read off the handshake files instead; only how an attempt ended has to be
# remembered here, because those files are gone by the time anyone asks.
login_flow = {'phase': 'idle', 'reason': None, 'codes': 0}

LOG_FIELDS = {'run_id', 'phase', 'outcome', 'aadsts', 'http_status', 'duration_ms'}


def log(event, **kw):
    """Whitelisted structured log — codes and cookies can never leak through it."""
    row = {'ts': datetime.now(timezone.utc).isoformat(timespec='seconds'),
           'event': event, **{k: v for k, v in kw.items() if k in LOG_FIELDS}}
    with (DATA / 'app-log.jsonl').open('a') as f:
        f.write(json.dumps(row) + '\n')


LOG_KEEP = 50          # data/ is a container volume, not infinite


def new_run_id():
    """A run's identity, minted by whoever opens its log file — which is this
    process, not the runner: a subprocess cannot name a file that was already
    open before it started.

    Time-ordered and filename-safe, so sorting the archive by name sorts it by
    age, which is what the cap prunes on. Unique because only this function
    mints them and it is called under proc_lock.
    """
    return datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S-%f')


def prune_logs():
    """Oldest archived logs dropped on write, so the volume cannot grow forever."""
    for old in sorted(RUN_LOGS.glob('*.log'))[:-LOG_KEEP]:
        old.unlink(missing_ok=True)


def run_log_file(run_id):
    """One run's archived log, or None.

    The id comes off a URL, and it is never joined onto a path — it is matched
    against the names that are actually there, so no input can walk out of the
    archive directory however it is spelled.
    """
    if not run_id:
        return None
    return next((p for p in RUN_LOGS.glob('*.log') if p.stem == run_id), None)


def newest_run_log():
    logs = sorted(RUN_LOGS.glob('*.log'))       # ids sort by age
    return logs[-1] if logs else None


# --------------------------------------------------------- where a run got to
#
# A run keeps no progress record of its own: it prints. sync.py announces every
# script it starts, the export prints the section list, a page count per section
# and one line per page it staged, the conversion prints one line per file, and
# the vault delta prints its three totals. Counting those lines is the whole
# progress display — which is why the two scripts that drive the client's
# SharePoint and vault did not have to be touched to build it.
#
# The exact wording of those print() calls is therefore an input to this file,
# and the suite pins every phrase read below: editing one of them fails pytest
# instead of quietly emptying the run page.

PHASE_ORDER = ('export', 'convert', 'delta', 'commit')
PHASE_LABEL = {'export': 'export', 'convert': 'convert',
               'delta': 'vault delta', 'commit': 'commit'}
PHASE_HEADLINE = {'export': 'Scanning OneNote',
                  'convert': 'Converting pages to markdown',
                  'delta': 'Working out what changed in the vault',
                  'commit': 'Writing to the vault'}

RAILS_SECTIONS = re.compile(r'^rails sections \((\d+)\):')
SECTION_PAGES = re.compile(r'^ {4}(\d+) pages$')
SECTION_SKIPPED = re.compile(r'^ {2}skip .+ — unchanged upstream \((\d+) staged pages\)$')
PAGES_STAGED = re.compile(r'^(\d+)/(\d+) pages staged in ')
CONVERTED_FILE = re.compile(r'^(?:ok  |LOSS) .*lost=')
CONVERT_DONE = re.compile(r'^(\d+) converted, worst coverage')
VAULT_DELTA = re.compile(r'^added (\d+) · changed (\d+) · removed (\d+)$')
REMOVALS_SUPPRESSED = re.compile(r'^ {2}! partial export \((\d+)/(\d+)\) — deletions skipped')
COMMITTED = re.compile(r'^committed (\S+): ')

# Why a page that was deleted upstream can still be sitting in the vault. The
# reader is otherwise left thinking the sync is broken.
SUPPRESSED_TEXT = ('removals suppressed: the export saw {seen} pages and rendered '
                   '{exported}, and a page that failed to render cannot be told apart '
                   'from one deleted upstream — so nothing was deleted from the vault '
                   'this run.')


def run_progress(text, live=False):
    """Where a run got to, counted off the lines it printed.

    None when the log carries no phase header at all: a sign-in prints none,
    writes no history row, and gets no run page — its state belongs to the
    Microsoft session card.
    """
    phase = None
    sections = seen_total = staged_total = None
    skipped = crawled = seen = exported = converted = 0
    delta = commit = note = None
    ended = False
    for line in text.splitlines():
        if line.startswith('--- export.py'):
            phase = 'export'
        elif line.startswith('--- convert.py'):
            phase = 'convert'
        elif CONVERTED_FILE.match(line):
            converted += 1
        elif phase == 'export' and line.startswith('    ok '):
            # only while the export is running: the vault delta lists its files at
            # the same indent, so a section named "ok …" would otherwise be counted
            # as a page staged after the export was already over
            exported += 1
        elif m := SECTION_PAGES.match(line):
            seen += int(m.group(1))
            crawled += 1
        elif m := SECTION_SKIPPED.match(line):
            seen += int(m.group(1))
            exported += int(m.group(1))
            skipped += 1
            crawled += 1
        elif m := RAILS_SECTIONS.match(line):
            sections = int(m.group(1))
        elif m := PAGES_STAGED.match(line):
            # authoritative: it counts the staged pages of skipped sections too
            exported, seen_total = int(m.group(1)), int(m.group(2))
            staged_total = exported
        elif m := CONVERT_DONE.match(line):
            converted, phase = int(m.group(1)), 'delta'
        elif m := VAULT_DELTA.match(line):
            delta = (f'{m.group(1)} added · {m.group(2)} changed · '
                     f'{m.group(3)} removed')
            phase = 'commit'
        elif m := REMOVALS_SUPPRESSED.match(line):
            note = SUPPRESSED_TEXT.format(exported=m.group(1), seen=m.group(2))
        elif m := COMMITTED.match(line):
            commit, phase, ended = m.group(1), 'commit', True
        elif line in ('nothing to commit', 'dry run — vault files updated, no commit'):
            phase, ended = 'commit', True
    if phase is None:
        return None

    # The denominator is the number of pages the export saw, and that is not
    # known until every section has been crawled — so before then the page
    # reports counters and says the percentage is still pending. A bar that
    # jumps backwards as a new section appears is worse than no bar.
    if phase == 'export':
        done = exported
        total = (seen_total if seen_total is not None
                 else seen if sections and crawled >= sections else None)
    else:
        done, total = converted, staged_total
    at = PHASE_ORDER.index(phase)
    counters = []
    if sections is not None:
        counters.append(f'{sections} sections found'
                        + (f' · {skipped} skipped as unchanged upstream' if skipped else ''))
    counters.append(f'{exported} pages exported'
                    + (f' · {max(total - exported, 0)} remaining'
                       if phase == 'export' and total else
                       f' of {seen_total} seen' if seen_total and seen_total != exported
                       else ''))
    if phase != 'export':
        counters.append(f'{converted} pages converted')
    return {
        'phase': phase,
        'phase_label': PHASE_LABEL[phase],
        'headline': ('Finished' if ended else PHASE_HEADLINE[phase] if live
                     else 'Stopped before finishing'),
        'phases': [{'key': k, 'label': PHASE_LABEL[k],
                    'state': ('done' if ended or i < at else
                              'active' if live else 'stopped') if i <= at else 'pending'}
                   for i, k in enumerate(PHASE_ORDER)],
        'done': done,
        'total': total,
        # capped: the numerator is counted off log lines and the denominator is
        # printed once, so a staging directory holding more than this run staged
        # must not read as 103%
        'percent': min(100, round(100 * done / total)) if total else None,
        # the honest state before the denominator exists: still counting, no number
        'indeterminate': total is None and not ended,
        'counters': counters,
        'delta': delta,
        'commit': commit,
        'note': note,
        'ended': ended,
    }


def live_progress():
    """The one line the dashboard carries while a run is in flight, linking to
    that run's page. The timeline, the bar and the counters have exactly one
    implementation, and it is not here."""
    log_file = newest_run_log()
    if log_file is None:
        return None
    try:
        # errors='replace': a log caught mid-write can split a multibyte page
        # title across the last flush, and a decode error here would take the
        # whole dashboard poll down with it
        prog = run_progress(log_file.read_text(errors='replace'), live=True)
    except OSError:
        return None
    if prog is None:
        return None
    count = f'{prog["done"]}/{prog["total"]}' if prog['total'] else f'{prog["done"]} so far'
    return {'run_id': log_file.stem,
            'text': f'running — {prog["phase_label"]}, {count}'}


def spawn(args, kind):
    """Start sync.py / export.py in the background; stdout tails into the UI."""
    global proc
    with proc_lock:
        if proc and proc.poll() is None:
            return False
        run_id = new_run_id()
        # the runner puts the same id on its history row, so the row and the log
        # agree on one identity and each log survives the next run
        env = {**os.environ, 'PYTHONUNBUFFERED': '1', 'RUN_ID': run_id}
        RUN_LOGS.mkdir(parents=True, exist_ok=True)
        out = (RUN_LOGS / f'{run_id}.log').open('w')
        prune_logs()
        proc = subprocess.Popen([sys.executable, *args], cwd=ROOT,
                                stdout=out, stderr=subprocess.STDOUT, env=env)
        threading.Thread(target=_reap, args=(proc, kind, time.time()),
                         daemon=True).start()
        log(f'{kind}_started', run_id=run_id)
        return True


def _reap(p, kind, t0):
    rc = p.wait()
    # the outcome first: forget_session_probe can block for as long as a probe
    # already in flight, and for that whole time the card would have its buttons
    # back while the sign-in still reads as running
    if kind == 'login':
        finish_login(rc)
    forget_session_probe()      # the run may have replaced the saved session
    log(f'{kind}_finished', outcome=f'rc_{rc}', duration_ms=int((time.time() - t0) * 1000))


def probe_session():
    """~200 ms instead of launching Chromium: call SharePoint with the saved cookies."""
    if not SITE:
        return {'ok': False, 'reason': 'SHAREPOINT_SITE is not set'}
    if not STATE.exists():
        return {'ok': False, 'reason': 'no storage-state.json'}
    try:
        cookies = json.loads(STATE.read_text())['cookies']
    except Exception:
        return {'ok': False, 'reason': 'unreadable storage state'}
    jar = '; '.join(f"{c['name']}={c['value']}" for c in cookies
                    if 'sharepoint.com' in c.get('domain', ''))
    req = urllib.request.Request(
        SITE + '/_api/web/currentUser',
        headers={'Accept': 'application/json;odata=nometadata', 'Cookie': jar})
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            who = json.load(r)
            return {'ok': True, 'http_status': r.status,
                    'user': who.get('Title') or who.get('Email') or '?'}
    except urllib.error.HTTPError as e:
        return {'ok': False, 'http_status': e.code}
    except Exception as e:      # redirect to the login page lands here (not JSON)
        return {'ok': False, 'reason': type(e).__name__}


PROBE_TTL = 60
_probe_cache = (0.0, None)     # (fetched_at, result)
probe_lock = threading.Lock()


def forget_session_probe():
    """Drop the cached answer because something just changed it.

    A finished sign-in makes the cached "re-login needed" a lie, and that lie is
    on the one card with a button next to it: the human clicks Re-login again and
    spends another of the five attempts the account gets per day. Taking the lock
    also means a probe already in flight cannot write its pre-login answer back on
    top of this.
    """
    global _probe_cache
    with probe_lock:
        _probe_cache = (0.0, None)


def probe_cached():
    """The dashboard used to probe once per page render; the SPA asks for state
    every few seconds, and a 10 s SharePoint call per tick would be a beating.
    One round trip a minute is plenty for a card that says "session alive".
    """
    global _probe_cache
    at, val = _probe_cache
    if val is not None and time.time() - at <= PROBE_TTL:
        return val
    # One probe in flight at a time. The cache is only stamped once urlopen
    # returns, so without this every tick arriving during a slow probe starts its
    # own: an unreachable SharePoint burns the full 10 s timeout against a 4 s
    # poll, which is ceil(10/4) = 3 threads per open tab out of waitress's 8 —
    # the admin would starve exactly when it is needed to say the session died.
    # A caller that finds a probe running reuses the stale answer rather than
    # queueing behind the network; only the very first has nothing to reuse.
    if not probe_lock.acquire(blocking=val is None):
        return val
    try:
        at, val = _probe_cache          # it may have landed while we waited
        if val is None or time.time() - at > PROBE_TTL:
            val = probe_session()
            _probe_cache = (time.time(), val)
    finally:
        probe_lock.release()
    return val


def read_json(path, default=None):
    """Sentinel files vanish, are half-written, or get hand-edited into the
    wrong shape — a dashboard render must never 500 on any of that."""
    try:
        v = json.loads(path.read_text())
    except (OSError, ValueError):
        return default
    # the default names the shape callers expect; None means a dict
    want = dict if default is None else type(default)
    return v if isinstance(v, want) else default


def account_email():
    """The Microsoft account a sign-in will use: the one saved here, else the
    deployment's env var. export.py reads the same file with the same fallback —
    the runner must never sign in as an account other than the one the card names.

    Lower-cased for the same reason the runner does it: the daily allowance is
    counted under this string, and Microsoft does not distinguish the capitals, so
    one mailbox must resolve to one bucket however it was typed.
    """
    saved = (read_json(ACCOUNT) or {}).get('email')
    return ((saved if isinstance(saved, str) else None)
            or os.environ.get('MS_EMAIL') or '').strip().lower()


def valid_email(value):
    """A trust boundary: this string becomes the account browser automation signs
    in as, it is written where the runner reads it, and it is the key the daily
    sign-in allowance is counted under — so one mailbox must not be able to
    present itself as two.

    Printable ASCII only, and no spaces. That is a deliberate trade: it refuses
    internationalised addresses, which a single-tenant admin signing into a
    corporate Microsoft account does not have, and in exchange it refuses the
    invisible characters that otherwise sail through — a zero-width space, a soft
    hyphen, a bidi mark or DEL is neither whitespace nor below 32, so each one
    would open its own fresh allowance against the same real mailbox.
    """
    local, _, domain = value.partition('@')
    return (bool(local) and '.' in domain and len(value) <= 200
            and value.isascii() and value.isprintable() and ' ' not in value)


def login_budget(email):
    """What is left of one account's daily sign-in allowance.

    The log is keyed by account — each one carries its own three — and a flat list
    written before that split counts against whoever asks until it ages out.
    """
    per_account = read_json(ATTEMPTS, {})
    tries = per_account.get(email) if per_account else read_json(ATTEMPTS, [])
    # read_json only vouches for the top level, and this file is hand-editable:
    # a scalar where a list belongs must not take the whole admin down with it
    if not isinstance(tries, list):
        tries = []
    used = len([t for t in (tries or [])
                if isinstance(t, (int, float)) and time.time() - t < 86_400])
    return {'used': used, 'left': max(0, LOGIN_BUDGET - used),
            'text': f'{used} of {LOGIN_BUDGET} daily login attempts used'}


# rc -> what to say when the runner left no reason of its own (killed, crashed)
LOGIN_RC = {7: 'blocked — the account is locked out',
            2: 'the sign-in did not go through — see the last run log'}


def finish_login(rc):
    """Remember how a sign-in ended, in the words the card shows."""
    reason = (read_json(LOGIN_RESULT) or {}).get('reason')
    if not isinstance(reason, str):
        reason = None
    login_flow.update(
        phase='succeeded' if rc == 0 else 'failed', codes=0,
        reason=None if rc == 0 else
        (reason or LOGIN_RC.get(rc) or 'the sign-in failed — see the last run log'))


def login_state():
    """Where a sign-in is, in six states: idle, starting, awaiting_code, verifying,
    succeeded, failed — the last one carrying the reason.

    Published in /status.json (and on /login-state.json), so the card can show the
    step it is on instead of the human guessing from a spinner.
    """
    st = {'phase': login_flow['phase'], 'reason': login_flow['reason'], 'rejected': False}
    if not (proc is not None and proc.poll() is None):
        return st                     # nothing running: idle, or how the last one ended
    pend = read_json(PENDING) if PENDING.exists() else None
    if pend is not None:
        # a code prompt is a sign-in in progress whoever started it: a scheduled
        # run parks on the same screen and also needs a human to type the code
        st.update(phase='verifying' if OTP_ANSWER.exists() else 'awaiting_code',
                  reason=None, rejected=bool(pend.get('rejected')))
    elif st['phase'] == 'starting' and login_flow['codes']:
        st['phase'] = 'verifying'     # the code was taken, the runner has not answered yet
    return st


def last_login_result():
    """Newest login_finished row from the app log — a failed re-login must be visible."""
    log_file = DATA / 'app-log.jsonl'
    if not log_file.exists():
        return None
    for line in reversed(log_file.read_text().splitlines()[-400:]):
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if isinstance(row, dict) and row.get('event') == 'login_finished':
            return row
    return None


def read_runs():
    """The whole history, oldest first."""
    if not RUNS.exists():
        return []
    rows = []
    for l in RUNS.read_text().splitlines():
        try:
            row = json.loads(l)
        except ValueError:       # a line still being appended
            continue
        if isinstance(row, dict):    # hand-edited scalar rows must not 500 the page
            rows.append(row)
    return rows


def tail_runs(n=20):
    return read_runs()[-n:][::-1]


def write_runs(rows):
    """Replace the history in one step.

    A reader sees either the whole old file or the whole new one — writing in
    place would hand it a truncated line, and a rewrite that dies halfway would
    leave that behind for good.

    The scratch file carries the writing thread's name, so this holds on its own
    without depending on the caller's lock: two writers filling and renaming one
    shared file would leave the loser pouring its rows through a handle on a file
    that is already the live history.
    """
    tmp = RUNS.with_name(f'{RUNS.name}.{threading.get_ident()}.tmp')
    try:
        tmp.write_text(''.join(json.dumps(r) + '\n' for r in rows))
        tmp.replace(RUNS)
    finally:
        tmp.unlink(missing_ok=True)


def edit_history(edit):
    """Run `edit` unless a run is in flight. False when it was refused.

    A run appends its row as it finishes, and editing rewrites the whole file — so
    a rewrite landing between two appends is the one way a row can silently vanish.
    Refusing for the length of a run closes that entirely, and a run is a minute.

    Deciding and editing happen under the very lock that starting a run takes,
    because they have to be one step: a run that begins in the gap has already
    opened its log, and the edit would unlink it and leave the run pouring its
    output into a file nobody can read. That one lock also serialises two people
    editing at the same moment, which would otherwise drop whichever rows the
    other had decided to keep. Both are button clicks, never a hot path, and
    nothing slow runs under this lock.
    """
    with proc_lock:
        if proc is not None and proc.poll() is None:
            return False
        edit()
        return True


def fmt_h(hours):
    """20.0 -> '20', 2.5 -> '2.5' — nobody says 'twenty point zero hours'."""
    return f'{hours:.1f}'.rstrip('0').rstrip('.')


def ago(iso):
    """'2026-07-31T09:58:06+00:00' -> '2 h ago' (compact, for the runs table)."""
    try:
        dt = datetime.fromisoformat(iso)
    except Exception:
        return iso
    if dt.tzinfo is None:            # hand-written timestamp — treat as UTC
        dt = dt.replace(tzinfo=timezone.utc)
    s = (datetime.now(timezone.utc) - dt).total_seconds()
    if s < 90:
        return 'just now'
    if s < 5400:
        return f'{int(s // 60)} min ago'
    if s < 129_600:
        # whole hours: '10.2 h ago' is a precision nobody reads an age to, and the
        # tenth is noise in front of the number that matters
        return f'{round(s / 3600)} h ago'
    return f'{int(s // 86400)} d ago'


# ------------------------------------------------------------------ schedule
#
# When the daily sync is due, and on whose clock. RUN_AT and the container's own
# zone stay the defaults, so a deployment that never saves a setting behaves
# exactly as it did before the setting existed. A saved zone is stored as a name
# rather than an offset, so the wall-clock time holds across a daylight-saving
# change — and the trigger, the once-per-day guard and the words in the UI all
# read that one clock. Read two, and the run either doubles up or goes missing on
# the day the two clocks disagree about the date.

def parse_hhmm(value):
    """'8:30' -> '08:30', anything else -> None.

    A trust boundary: this string is compared against a wall clock and shown to a
    human, and it arrives from a form and from an env var. Neither may put
    something else in front of either reader.
    """
    try:
        return datetime.strptime(value, '%H:%M').strftime('%H:%M')
    except (TypeError, ValueError):
        return None


def load_zone(name):
    """A zone name -> its rules. None means the container's own clock.

    An unknown name, a path-shaped one, and a tz database missing from the image
    all land here the same way: no zone, so the schedule keeps running on the
    clock it used before instead of the whole admin failing to render.

    Every exception, not a chosen few: this is a validator whose whole contract is
    "unusable name -> None", and the lookup reaches the import machinery, which
    has its own ways to fail (a name of several hundred path segments recurses).
    A stored name that raises here would otherwise 500 every dashboard poll and
    stop the daily sync for good.
    """
    try:
        return ZoneInfo(name) if name else None
    except Exception:
        return None


def now_in(tz):
    """The one seam onto the wall clock, so a test can stand at a chosen moment
    instead of waiting for one to come round."""
    return datetime.now(tz)


def due_moment(now, at):
    """Today's HH:MM on the clock `now` is already read in. Kept in one place
    because the saver and the reader must judge "has it gone by" identically."""
    hh, mm = at.split(':')
    return now.replace(hour=int(hh), minute=int(mm), second=0, microsecond=0)


def schedule_state():
    """The configured run time, the zone it is read in, that zone's today, and
    whether today's moment has come."""
    saved = read_json(SCHEDULE_SETTINGS) or {}
    at = parse_hhmm(saved.get('time')) or parse_hhmm(RUN_AT) or '08:30'
    tz = load_zone(saved.get('tz'))
    now = now_in(tz)
    today = now.date().isoformat()
    return {
        'time': at,
        # the name when there is one; otherwise the abbreviation of the clock
        # actually in use, because an unlabelled time is what this replaces
        'zone': saved.get('tz') if tz else now.astimezone().tzname(),
        # the saved name alone, so the editor can tell "configured" from
        # "whatever clock this container happens to keep"
        'zone_saved': saved.get('tz') if tz else None,
        'tz': tz,
        'today': today,
        'due': now >= due_moment(now, at),
        # saving a setting is not a Run now: a time that had already gone by when
        # it was saved first comes round the next day. Stamped as a date on the
        # configured clock rather than an epoch, so it is compared the same
        # wall-clock way `due` is — an offset that moves under a daylight-saving
        # change used to make the two disagree and let the save start a sync.
        'changed_since_due': saved.get('skipDate') == today,
    }


def local_ts(iso, tz):
    """An absolute timestamp on the configured clock, zone named — a bare UTC
    string is a time the reader cannot check against the clock on their wall."""
    try:
        dt = datetime.fromisoformat(iso)
    except (TypeError, ValueError):
        return iso
    if dt.tzinfo is None:            # hand-written timestamp — treat as UTC
        dt = dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(tz).strftime('%Y-%m-%d %H:%M %Z')


# ------------------------------------------------------------------- wording
#
# Everything a human reads is worded here, server-side, and published in
# /status.json. The browser paints the strings it is handed and never maps a raw
# outcome code to a phrase itself — one copy of the vocabulary, not two.

def human_outcome(row):
    """Words for the UI; the raw code stays in runs.jsonl and /log."""
    o = str((row or {}).get('outcome') or 'never ran')
    if o == 'committed':
        n = (row.get('added') or 0) + (row.get('changed') or 0)
        return (f'synced {n} page{"s" if n != 1 else ""}' if n
                else 'synced — removals only' if row.get('removed')
                else 'synced — vault maintenance'), 'ok'
    # the commit is in the container and nowhere else: every other reader of this
    # vault — a laptop, the next agent — is looking at stale specs
    if o == 'push_failed':
        return 'committed but not pushed — GitHub is behind', 'bad'
    if o == 'no_changes':
        return 'no changes upstream', 'ok'
    if o in ('paused', 'export_rc_7'):
        return 'paused', 'mut'
    if o == 'export_rc_2':
        return 'session dead — re-login needed', 'bad'
    if o == 'convert_failed':
        return 'conversion check failed — see log', 'bad'
    if o == 'pull_failed':
        return 'vault conflict — fix by hand', 'bad'
    if o == 'never ran':
        return o, 'mut'
    return 'failed — see log', 'bad'


def probe_human(sess):
    """The session card speaks in next actions, not HTTP codes or exceptions."""
    reason = sess.get('reason', '')
    if reason == 'SHAREPOINT_SITE is not set':
        return 'not configured — set SHAREPOINT_SITE'
    if reason == 'no storage-state.json':
        return 'no saved session yet — re-login needed'
    if reason == 'unreadable storage state':
        return 'saved session unreadable — re-login needed'
    if sess.get('http_status') in (401, 403):
        return 'session rejected — re-login needed'
    if reason == 'JSONDecodeError':      # login redirect answers HTML, not JSON
        return 'session expired — re-login needed'
    return 'SharePoint unreachable'


def worded(row, tz=None):
    """A run row plus the words shown for it: the badge, its age, the moment it
    started on the configured clock, and the added/changed/removed sentence the
    last-sync card reads out."""
    # only a missing row has no words; an empty or half-filled one still has to
    # come back as a row, because callers scan these for an outcome and a
    # hand-edited `{}` line must not take the whole admin down
    if row is None:
        return None
    text, kind = human_outcome(row)
    counts = (f'{row.get("added", 0)} added · {row.get("changed", 0)} changed · '
              f'{row.get("removed", 0)} removed'
              if row.get('outcome') in ('committed', 'push_failed')
              else 'nothing changed upstream')
    return {**row, 'outcome_text': text, 'outcome_kind': kind,
            'ago': ago(row.get('started', '')),
            'when': local_ts(row.get('started', ''), tz), 'counts': counts}


def last_login_human():
    """A pause no longer stops a sign-in, so the only pause left that can is the
    lockout — say so, instead of pointing at the Resume button."""
    row = last_login_result()
    if not row:
        return None
    outcome = row.get('outcome')
    return {'text': ('succeeded' if outcome == 'rc_0'
                     else 'blocked — the account is locked out' if outcome == 'rc_7'
                     else 'failed — see log'),
            'ago': ago(row.get('ts', ''))}


def health_pill(s, sess):
    """The top line, five states. It is the one number that makes silent
    breakage visible, so it gets worded once, here."""
    if s['running']:
        return {'text': 'task running', 'kind': 'run'}
    if s['paused']:
        return {'text': 'paused', 'kind': 'bad'}
    if s['healthy'] and not sess['ok']:
        return {'text': 'specs fresh · session dead — next run will fail', 'kind': 'warn'}
    if s['healthy']:
        # the same wording as every other age on these screens, ago()'s: rounding
        # the hours here said '0 h ago' for a run half an hour old, and '10.2 h
        # ago' before that — a precision nobody reads an age to
        return {'text': f'specs fresh · {s["last_success"]["ago"]}', 'kind': 'ok'}
    return {'text': f'stale · {human_outcome(s["last_run"])[0]}', 'kind': 'bad'}


def status():
    sched = schedule_state()
    runs = [worded(r, sched['tz']) for r in tail_runs()]
    last_ok = next((r for r in runs if r.get('outcome') in ('committed', 'no_changes')), None)
    age_h = None
    if last_ok:
        started = datetime.fromisoformat(last_ok['started'])
        age_h = (datetime.now(timezone.utc) - started).total_seconds() / 3600
    email = account_email()
    budget = login_budget(email)
    running = proc is not None and proc.poll() is None
    # a code prompt is only real while its consumer lives: a runner killed
    # mid-login (redeploy, crash) leaves pending.json behind, and a banner with
    # no reader swallows the code into the void
    pending = None
    if running:
        pending = read_json(PENDING)
    elif PENDING.exists():
        PENDING.unlink(missing_ok=True)
    paused = read_pause(PAUSED)
    try:
        state_age_h = round((time.time() - STATE.stat().st_mtime) / 3600, 1)
    except OSError:
        state_age_h = None
    sess = probe_cached()
    sched_done = (read_json(SCHEDULE) or {}).get('lastRunDate')
    s = {
        'healthy': age_h is not None and age_h < 26,
        'last_success': last_ok,
        'last_success_age_hours': round(age_h, 1) if age_h is not None else None,
        # the newest history row *is* the last run, so it is read from the rows
        # the table shows: a separate copy would keep reporting a run whose row
        # was just deleted, which is exactly the stale state a clear must not
        # leave. Those rows are already worded on the configured clock above.
        'last_run': runs[0] if runs else None,
        'running': running,
        'paused': paused,
        'paused_since': ago(paused['at']) if paused and paused.get('at') else None,
        'pending': pending,
        'account_email': email,
        'login_budget': budget,
        'login': login_state(),
        'login_attempts_24h': budget['used'],
        'storage_state_age_hours': state_age_h,
        'runs': runs,
        # one line, and a link through to the run's own page
        'run_progress': live_progress() if running else None,
        # a paused schedule still runs on demand; only a lockout takes Run now
        # away, because that run would die on the sign-in anyway
        'lockout': is_lockout(paused),
        'run_at': sched['time'],
        'timezone': sched['zone'],
        'timezone_saved': sched['zone_saved'],
        'next_run': ('paused' if paused
                     else 'done for today, next tomorrow'
                     if sched_done == sched['today']
                     else 'later today' if not sched['due']
                     else 'first run at the new time is tomorrow'
                     if sched['changed_since_due']
                     else 'within a minute'),
        'vault_web': VAULT_WEB,
        'session': {
            'ok': sess['ok'],
            'user': sess.get('user'),
            'phrase': None if sess['ok'] else probe_human(sess),
            'state_age': ('no saved session' if state_age_h is None
                          else f'saved session {fmt_h(state_age_h)} h old'),
        },
        'last_login': last_login_human(),
        'notice': None,
    }
    s['health'] = health_pill(s, sess)
    return s


def state_json(notice=None):
    """The single response shape: /status.json and every action POST answer it,
    so the browser has one parser and one state setter."""
    return {**status(), 'notice': notice}


# ---------------------------------------------------------------- auth & shell

# The sign-in form is the only HTML this module still renders, so this is only
# what that one card needs. The bundle's Mantine theme carries the same light/dark
# hexes in its own copy; nothing links them, so a palette change means both.
STYLE = """
:root{--bg:#f4f6f9;--card:#fff;--txt:#1b2430;--mut:#68737f;--line:#e2e7ee;
--red:#b42318;--btn:#1b2430;--btn-txt:#fff}
@media(prefers-color-scheme:dark){:root{--bg:#0f141b;--card:#161d27;--txt:#e7ecf2;
--mut:#8b97a5;--line:#27313d;--red:#f87171;--btn:#e7ecf2;--btn-txt:#0f141b}}
*{box-sizing:border-box}
body{font:15px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif;background:var(--bg);
color:var(--txt);margin:0;padding:22px}
main{max-width:900px;margin:0 auto}
h1{font-size:19px;margin:0;letter-spacing:-.01em}
.card{background:var(--card);border:1px solid var(--line);border-radius:13px;padding:15px 16px}
.mut{color:var(--mut);font-size:13px}
button{background:var(--btn);color:var(--btn-txt);border:0;border-radius:8px;
padding:8px 14px;font:600 13.5px system-ui;cursor:pointer}
button:hover{opacity:.85}
input[type=password]{background:var(--bg);border:1px solid var(--line);color:var(--txt);
border-radius:8px;padding:8px 11px;font:14px system-ui}
"""


FAVICON = ('data:image/svg+xml,<svg xmlns=%27http://www.w3.org/2000/svg%27 '
           'viewBox=%270 0 100 100%27><text y=%27.9em%27 font-size=%2790%27>📓</text></svg>')


def page(body):
    return ('<!doctype html><meta charset="utf-8">'
            '<meta name="viewport" content="width=device-width,initial-scale=1">'
            '<title>onenote-md-sync</title>'
            f'<link rel="icon" href="{FAVICON}">'
            f'<style>{STYLE}</style><main>{body}</main>')


def login_page(err=False):
    msg = '<p style="color:var(--red);margin:10px 0 0">Wrong token, try again</p>' if err else ''
    return page(f'''
<div class="card" style="max-width:360px;margin:16vh auto 0">
  <h1 style="margin-bottom:2px">onenote-md-sync</h1>
  <p class="mut" style="margin-top:2px">OneNote notebook &rarr; Markdown vault</p>
  <form method="post" action="/auth" style="display:flex;gap:8px;margin-top:14px">
    <input type="password" name="token" placeholder="admin token" autofocus
           style="flex:1" autocomplete="current-password">
    <button>Sign in</button>
  </form>{msg}
</div>''')


SESSION_TTL = 30 * 86_400


def session_mac(exp):
    """Derived from the token, never the token itself; the expiry is baked
    into the MAC so a stolen cookie ages out without any server-side state."""
    return hmac.new(TOKEN.encode(), f'admin-session-v1|{exp}'.encode(),
                    'sha256').hexdigest()


def session_cookie():
    exp = int(time.time()) + SESSION_TTL
    return f'{exp}.{session_mac(exp)}'


def session_valid(value):
    exp, _, mac = value.partition('.')
    if not exp.isdigit() or int(exp) <= time.time():
        return False
    return hmac.compare_digest(mac.encode(), session_mac(int(exp)).encode())


FAILURES = {}   # ip -> recent wrong-token timestamps; ponytail: in-memory, resets on restart
fail_lock = threading.Lock()
FAIL_CAP, FAIL_WINDOW = 10, 900
THROTTLED = 'too many wrong tokens — wait a few minutes and retry'


def client_ip():
    # rightmost X-Forwarded-For hop: Traefik appends the real client IP, so
    # anything left of it is client-supplied and spoofable
    fwd = request.headers.get('X-Forwarded-For', '')
    return fwd.rsplit(',', 1)[-1].strip() or request.remote_addr or '?'


def too_many_failures(ip):
    with fail_lock:
        now = time.time()
        recent = [t for t in FAILURES.get(ip, []) if now - t < FAIL_WINDOW]
        if recent:
            FAILURES[ip] = recent
        else:
            FAILURES.pop(ip, None)      # quiet IPs must not accumulate forever
        return len(recent) >= FAIL_CAP


def note_failure(ip):
    # no in-thread sleep: a tarpit would hold a waitress worker per wrong
    # token, and 8 of those freeze the whole admin — the cap does the braking
    with fail_lock:
        FAILURES.setdefault(ip, []).append(time.time())
    log('auth_failed')


def authed():
    header = request.headers.get('X-Admin-Token')
    if header is not None:           # automation path: the raw token, throttled
        ip = client_ip()
        if too_many_failures(ip):
            return False
        if hmac.compare_digest(header.encode(), TOKEN.encode()):
            return True
        note_failure(ip)
        return False
    return session_valid(request.cookies.get('t', ''))


def spa_route(path):
    """Whether a GET path is one the client-side router owns — the tab pages and
    a single run's page. The tab pages are listed, so a typo stays a 404; a run id
    is not checked here, because whether it has a log is /run.json's answer."""
    p = path.strip('/')
    return p in ('', *SPA_PAGES) or (p.startswith('runs/') and p.count('/') == 1)


def cross_site():
    """CSRF second layer behind SameSite=Lax: browsers stamp cross-site POSTs
    with Origin / Sec-Fetch-Site; requests without them (curl) pass."""
    origin = request.headers.get('Origin', '')
    if origin and origin.rsplit('://', 1)[-1] != request.host:
        return True
    return request.headers.get('Sec-Fetch-Site', 'same-origin') not in ('same-origin', 'none')


@app.before_request
def auth():
    if not TOKEN:
        return Response('ADMIN_TOKEN is not set — refusing everything', 503)
    if request.method == 'POST' and cross_site():
        return Response('cross-site request rejected', 403)
    # the exemption is for submitting the form, nothing else: anything mounted at
    # /auth for another method must not be unauthenticated by construction
    if (request.path == '/auth' and request.method == 'POST') or authed():
        return None
    # a throttled IP gets the truth: a 403 here reads as "wrong token" to
    # automation even when the token is right
    if request.headers.get('X-Admin-Token') is not None and too_many_failures(client_ip()):
        return Response(THROTTLED, 429)
    # any page the SPA router owns is a URL a human can have bookmarked, so an
    # expired session there must land on the sign-in form, not a bare 403
    if request.method == 'GET' and spa_route(request.path):
        return login_page()
    return Response('unauthorized', 403)


def _secure():
    return request.headers.get('X-Forwarded-Proto', request.scheme) == 'https'


@app.post('/auth')
def auth_submit():
    ip = client_ip()
    if too_many_failures(ip):
        return Response(THROTTLED, 429)
    if hmac.compare_digest(request.form.get('token', '').encode(), TOKEN.encode()):
        resp = redirect('/')
        resp.set_cookie('t', session_cookie(), httponly=True, samesite='Lax',
                        secure=_secure(), max_age=SESSION_TTL)
        return resp
    note_failure(ip)
    return login_page(err=True)


@app.post('/logout')
def logout():
    resp = redirect('/')
    resp.delete_cookie('t')
    return resp


@app.after_request
def no_store(resp):
    # a public admin: nothing may survive in shared caches or on disk
    resp.headers['Cache-Control'] = 'no-store'
    # send_file stamps these on the shell and the hashed assets; they are
    # validators for a stored copy no cache is allowed to keep, so they only
    # muddle the instruction above
    resp.headers.pop('ETag', None)
    resp.headers.pop('Last-Modified', None)
    resp.headers['X-Content-Type-Options'] = 'nosniff'
    # the vault links open github.com; the admin's hostname is not github's business.
    # NOT 'no-referrer': that makes browsers stamp navigational form POSTs with
    # Origin: null, which cross_site() rejects, so nobody can sign in or sign out.
    # 'same-origin' still sends no referrer off-site, which is the whole point.
    resp.headers['Referrer-Policy'] = 'same-origin'
    return resp


# ------------------------------------------------------------------- actions

# Every action answers the state that resulted from it, so the browser applies
# one response instead of redirecting and re-fetching the whole page.

@app.get('/status.json')
def status_json():
    return state_json()


@app.get('/log')
def run_log():
    """The last run's log, or ?id=<run_id> for one particular run's."""
    wanted = request.args.get('id', '')
    path = run_log_file(wanted) if wanted else newest_run_log()
    if wanted and path is None:
        return Response('no log kept for that run', 404, mimetype='text/plain')
    text = path.read_text() if path else ''
    return Response(text[-20_000:], mimetype='text/plain')


@app.get('/run.json')
def run_json():
    """One run: where it got to, how it was decided, and its log.

    The run page polls this while the run is live and stops once it settles, so
    `live` is the answer to "is this still moving" — a process that died leaves a
    log that would otherwise read as a run frozen mid-phase forever.

    A run can end before the export ever starts — a pause, or a vault pull that
    needs hands — and it still has a history row and a log saying what to do. So
    a missing progress block is not by itself a missing page: only a run with no
    row either was a sign-in, and only that gets the 404.
    """
    run_id = request.args.get('id', '')
    path = run_log_file(run_id)
    if path is None:
        return {'error': 'no log kept for that run'}, 404
    live = (proc is not None and proc.poll() is None and newest_run_log() == path)
    try:
        # errors='replace': a page title can be split across the last flush of a
        # log still being written, and that must not 500 the page
        text = path.read_text(errors='replace')
    except OSError:
        text = ''
    prog = run_progress(text, live)
    row = next((r for r in read_runs() if r.get('run_id') == run_id), None)
    if prog is None and row is None:
        return {'error': 'that run never reached the export — a sign-in has no '
                         'progress of its own, only the session card'}, 404
    return {'run_id': run_id, 'live': live, 'progress': prog,
            # the configured clock, not UTC: the history table words its rows on it
            # and this page shows the same moment — two zones for one run reads as
            # two different runs
            'run': worded(row, schedule_state()['tz']),
            # this page is the only log surface there is now, so the tail has to
            # be long enough to hold a whole run rather than /log's curl-sized cut
            'log': text[-200_000:]}


@app.post('/run')
def run_now():
    # asking for a run by hand overrides the pause on the schedule — that is the
    # whole point of the button. A lockout pause is not ours to override, and the
    # button is disabled then, so this only catches a POST sent directly.
    if not is_lockout(read_pause(PAUSED)):
        spawn(['sync.py', '--ignore-pause'], 'sync')
    return state_json()


# One-shot feedback that belongs to one action; it rides back in that action's
# own response, so a later poll or a reload cannot repeat it.
NO_LOGIN_NOTICE = {'kind': 'amber',
                   'text': 'No login is waiting for a code — the attempt may have '
                           'finished or timed out.'}
NO_EMAIL_NOTICE = {'kind': 'amber',
                   'text': 'Enter the Microsoft account email to sign in as first.'}
BAD_EMAIL_NOTICE = {'kind': 'amber', 'text': 'That does not look like an email address.'}
NO_BUDGET_NOTICE = {'kind': 'amber',
                    'text': 'No sign-in attempts left today for this account — '
                            'another account has its own.'}
LOCKED_OUT_NOTICE = {'kind': 'amber',
                     'text': 'The account is locked out — Microsoft clears that on its '
                             'own. Signing in again now only keeps it locked, so wait '
                             'it out.'}
BUSY_NOTICE = {'kind': 'amber', 'text': 'A task is already running — wait for it to finish.'}
HISTORY_BUSY_NOTICE = {'kind': 'amber',
                       'text': 'A run is writing to the history right now — wait for it '
                               'to finish, then try again.'}


@app.post('/login')
def login_now():
    """Start a sign-in, saving the account it is for on the way in: the address in
    the card is the one the runner will use, and one round trip sets both.

    Every refusal is answered before anything is written, and against the address
    this request is for. This is the only thing in the app that discards a stored
    session, so a request it is going to turn down must leave the live session, the
    saved account and the cached probe exactly as it found them.
    """
    # normalised on the way in, so the address the card shows, the address the
    # runner types and the bucket the allowance is counted under are one string
    email = request.form.get('email', '').strip().lower()
    if email and not valid_email(email):
        return state_json(BAD_EMAIL_NOTICE)
    target = email or account_email()
    if not target:
        return state_json(NO_EMAIL_NOTICE)
    # the runner is the authority on the allowance, but it only refuses after
    # truncating the run log and starting a browser, so refuse the spent account
    # here too rather than paying for an answer already known. Counted against the
    # address being signed in as, because the allowance is per account.
    if not login_budget(target)['left']:
        return state_json(NO_BUDGET_NOTICE)
    # a lockout is the one pause a hand-made request may not push through — the
    # runner refuses it too, but only from inside a spawn we would have paid for
    if is_lockout(read_pause(PAUSED)):
        return state_json(LOCKED_OUT_NOTICE)
    # asked before the session is touched; the spawn below refuses a second task
    # anyway, and that answer arrives too late to keep this one harmless
    if proc is not None and proc.poll() is None:
        return state_json(BUSY_NOTICE)
    if target != account_email():
        # and it lands before the spawn, because the runner reads this file to find
        # out whose sign-in it is
        ACCOUNT.write_text(json.dumps({'email': target}))
        # a session saved for the previous account cannot stand in for this one:
        # leaving it would skip the sign-in entirely and the card would report
        # itself signed in as an account it never authenticated
        STATE.unlink(missing_ok=True)
        forget_session_probe()
    # set before the spawn, or a runner that dies instantly reports its outcome
    # into a state that is then overwritten with 'starting'
    login_flow.update(phase='starting', reason=None, codes=0)
    try:
        started = spawn(['export.py', '--login-only'], 'login')
    except OSError:
        # a full disk or a fork that fails would otherwise leave the card saying
        # 'starting' for the life of the process, with every control spinning
        login_flow.update(phase='idle', reason=None)
        raise
    if not started:
        login_flow.update(phase='idle', reason=None)
        return state_json(BUSY_NOTICE)
    return state_json()


@app.get('/login-state.json')
def login_state_json():
    """The sign-in's state on its own, for anything that does not want the whole
    status document. The card reads the same block out of /status.json."""
    return login_state()


@app.post('/code')
def submit_code():
    code = request.form.get('code', '').strip()
    if not code:
        return state_json()
    # a code prompt is only real while its consumer lives — a dead runner
    # would swallow the code into the void with no feedback
    if not (proc is not None and proc.poll() is None and PENDING.exists()):
        return state_json(NO_LOGIN_NOTICE)
    OTP_ANSWER.write_text(json.dumps({'code': code}))
    # the runner deletes the prompt as it picks the code up, so without this the
    # card would fall back from "authorising" to "starting" until it answers
    login_flow['codes'] += 1
    return state_json()


@app.post('/runs/clear')
def clear_runs():
    """Throw the whole history away, rows and archived logs together.

    The health line derives from these rows, so an empty history honestly reads
    "never ran" until the next sync — that was agreed, not a bug to paper over.
    """
    def wipe():
        RUNS.unlink(missing_ok=True)
        LAST_RUN.unlink(missing_ok=True)
        for old in list(RUN_LOGS.glob('*.log')):
            old.unlink(missing_ok=True)

    # the lock covers the edit only, never the state that is read back afterwards:
    # answering the state can probe SharePoint, and no button should wait on that
    if not edit_history(wipe):
        return state_json(HISTORY_BUSY_NOTICE)
    return state_json()


@app.post('/runs/delete')
def delete_run():
    """Drop one row and its archived log.

    Named by run id, or by its `started` timestamp for the rows written before
    ids existed — those keep their place in the table and stay deletable.
    """
    run_id = request.form.get('id', '')
    started = request.form.get('started', '')
    if not run_id and not started:
        return state_json()

    def drop():
        rows = read_runs()
        keep = [r for r in rows
                if (r.get('run_id') != run_id if run_id else r.get('started') != started)]
        if len(keep) != len(rows):   # a stale table clicking a gone row changes nothing
            write_runs(keep)
        gone = run_log_file(run_id)
        if gone:
            gone.unlink(missing_ok=True)

    if not edit_history(drop):
        return state_json(HISTORY_BUSY_NOTICE)
    return state_json()


@app.post('/pause')
def pause():
    PAUSED.write_text(json.dumps({'reason': 'manual pause',
                                  'at': datetime.now(timezone.utc).isoformat()}))
    return state_json()


@app.post('/resume')
def resume():
    PAUSED.unlink(missing_ok=True)
    return state_json()


BAD_TIME_NOTICE = {'kind': 'amber', 'text': 'Enter the time as HH:MM, for example 08:30.'}
BAD_ZONE_NOTICE = {'kind': 'amber',
                   'text': 'This server does not know that timezone — pick one from the list.'}


@app.post('/schedule')
def set_schedule():
    """Save when the daily sync runs, and on whose clock.

    Saving never starts a run. If the time has already gone by today, the first
    run at it is tomorrow — the card says so, so an edit made at lunchtime cannot
    surprise anyone with a sync a second later.
    """
    at = parse_hhmm(request.form.get('time', '').strip())
    if not at:
        return state_json(BAD_TIME_NOTICE)
    zone = request.form.get('tz', '').strip()
    if zone and load_zone(zone) is None:
        return state_json(BAD_ZONE_NOTICE)
    now = now_in(load_zone(zone or None))
    gone_by = now >= due_moment(now, at)
    SCHEDULE_SETTINGS.write_text(json.dumps(
        {'time': at, 'tz': zone or None,
         'skipDate': now.date().isoformat() if gone_by else None}))
    return state_json()


# ----------------------------------------------------------------- the bundle

MISSING_BUNDLE = ('frontend/dist is missing — build it first:\n'
                  '    npm --prefix frontend install && npm --prefix frontend run build\n')


@app.get('/')
def index():
    """The SPA shell. before_request has already decided this visitor is signed
    in — an anonymous GET / never reaches here, it gets login_page() instead."""
    if not (DIST / 'index.html').exists():
        return Response(MISSING_BUNDLE, 503, mimetype='text/plain')
    return send_from_directory(DIST, 'index.html')


@app.get('/<page>')
def spa_page(page):
    """A client-side route reloaded in place still has to answer with the shell.
    Named explicitly rather than as a catch-all, so a typo stays a 404."""
    if page not in SPA_PAGES:
        return Response('not found', 404)
    return index()


@app.get('/runs/<run_id>')
def spa_run_page(run_id):
    """A run's page is a real URL, so it is bookmarkable and survives a reload.
    Whether that id has a log is the page's own question, asked of /run.json."""
    return index()


def scheduler():
    while True:
        try:
            sched = schedule_state()
            done = (read_json(SCHEDULE) or {}).get('lastRunDate')
            if (done != sched['today'] and sched['due'] and not PAUSED.exists()
                    and not sched['changed_since_due']):
                if spawn(['sync.py'], 'sync'):
                    SCHEDULE.write_text(json.dumps({'lastRunDate': sched['today']}))
        except Exception as e:
            log('scheduler_error', outcome=type(e).__name__)
        time.sleep(60)


if __name__ == '__main__':
    DATA.mkdir(exist_ok=True)
    threading.Thread(target=scheduler, daemon=True).start()
    # threaded WSGI: a slow SharePoint probe (10 s timeout) must not block
    # the next request the way the single-threaded dev server did
    from waitress import serve
    serve(app, host=os.environ.get('BIND', '127.0.0.1'),
          port=int(os.environ.get('PORT', 8377)), threads=8)
