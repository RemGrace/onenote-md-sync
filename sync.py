#!/usr/bin/env python3
"""One run: export -> convert -> copy changed notes into the vault -> commit.

    python sync.py                 # full run, commits if anything changed
    python sync.py --dry-run       # do everything except git add/commit/push
    python sync.py --no-push       # commit locally, do not push
    python sync.py --section Client --limit 3     # passed through to export.py

Delta lives in the vault at <VAULT_NOTES_DIR>/.sync-state.json, so losing this
container cannot desync the state from the output. Only paths under the specs
tree are ever touched; a human editing the vault in Obsidian is not overwritten
(a rebase conflict aborts the run instead).
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import unicodedata
from datetime import datetime, timezone
from pathlib import Path

from pause import blocks, read_pause

ROOT = Path(__file__).parent
OUT = ROOT / 'data' / 'out'
VAULT = Path(os.environ.get('VAULT_DIR') or Path.home() / 'vaults' / 'notes').expanduser()
SPECS = VAULT / os.environ.get('VAULT_NOTES_DIR', 'specs')   # the only tree a run touches
STATE_FILE = SPECS / '.sync-state.json'
LAST_RUN = ROOT / 'data' / 'last_run.json'
RUNS_LOG = ROOT / 'data' / 'runs.jsonl'
MAX_FILE_BYTES = 2 * 1024 * 1024        # a text vault should never carry more


def record(outcome, started, **extra):
    # the admin minted this id when it opened the log file for this run, so the
    # row and its log share one identity. A run started from a shell has no id
    # and its row simply carries none.
    # last, so no caller can hand in a run_id of its own: the log was already
    # opened under this one, and a row naming a different log is a dead link
    run_id = os.environ.get('RUN_ID')
    row = {'started': started, 'outcome': outcome, **extra,
           **({'run_id': run_id} if run_id else {})}
    LAST_RUN.write_text(json.dumps(row, indent=2))
    with RUNS_LOG.open('a') as f:
        f.write(json.dumps(row) + '\n')


def run(cmd, cwd, check=True):
    p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if check and p.returncode != 0:
        raise RuntimeError(f'{" ".join(cmd)} → {p.returncode}\n{p.stderr.strip()}')
    return p


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def push(vault, attempts=3):
    """Push, rebasing onto whatever landed while this run was working.

    A run takes minutes and this vault has other writers (a laptop in Obsidian,
    an agent writing session notes). One of them pushing between our pull and
    our push is ordinary, not exceptional — and our commit is the only copy of
    those pages, so the answer is to rebase onto them and push again. Never a
    force-push: their work is not ours to overwrite.

    Returns '' once the commit is on the remote, the git error otherwise.
    """
    err = ''
    for attempt in range(attempts):
        p = run(['git', 'push'], vault, check=False)
        if p.returncode == 0:
            return ''
        err = p.stderr.strip()
        if attempt == attempts - 1:
            break
        pull = run(['git', 'pull', '--rebase', '--autostash'], vault, check=False)
        if pull.returncode != 0:
            # a real conflict with someone's edit. Abort it: a half-finished
            # rebase reads as staged changes, and then no later run even gets
            # far enough to retry — it dies at the guard before the pull
            run(['git', 'rebase', '--abort'], vault, check=False)
            return pull.stderr.strip()
    return err


def stage_step(script, extra=()):
    cmd = [sys.executable, str(ROOT / script), *extra]
    print(f'--- {script} {" ".join(extra)}')
    p = subprocess.run(cmd, cwd=ROOT)
    return p.returncode


def collect(out_dir):
    """{rel_md_path: {'md_sha', 'images': [rel]}} for everything converted."""
    notes = {}
    for md in sorted(out_dir.rglob('*.md')):
        rel = md.relative_to(out_dir).as_posix()
        text = md.read_text()
        imgs = sorted(set(
            p for p in __import__('re').findall(r'!\[[^\]]*\]\(([^)]+)\)', text)))
        notes[rel] = {'md_sha': sha(md), 'images': imgs}
    return notes


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--dry-run', action='store_true')
    ap.add_argument('--no-push', action='store_true')
    ap.add_argument('--skip-export', action='store_true', help='reuse data/staging')
    ap.add_argument('--section')
    ap.add_argument('--limit', type=int)
    ap.add_argument('--ignore-pause', action='store_true',
                    help='a human asked for this run despite the pause (a lockout still stops it)')
    args = ap.parse_args()

    started = datetime.now(timezone.utc).isoformat(timespec='seconds')
    # the pause stops the schedule, not the operator: a run someone asked for by
    # hand goes through. A lockout pause is Microsoft's, and stops that too.
    paused = read_pause(ROOT / 'data' / 'PAUSED')
    if blocks(paused, by_hand=args.ignore_pause):
        print(f'PAUSED: {paused.get("reason", "paused")} — clear it in the admin',
              file=sys.stderr)
        record('paused', started)
        return 7
    if not SPECS.exists():
        print(f'vault specs tree not found at {SPECS}', file=sys.stderr)
        return 3

    # never start on top of someone's half-staged work (no .strip() on the whole
    # output: it eats the first line's leading space and ` M` reads as staged `M`)
    dirty = run(['git', 'status', '--porcelain'], VAULT).stdout
    staged = [l for l in dirty.splitlines() if l and l[0] not in ' ?']
    if staged:
        print('vault has staged changes — refusing to run:', *staged[:5], sep='\n  ')
        return 4

    # pull before writing anything, so our own fresh writes are never what the
    # rebase trips over; autostash absorbs leftovers of an interrupted run
    if not args.dry_run:
        pull = run(['git', 'pull', '--rebase', '--autostash'], VAULT, check=False)
        if pull.returncode != 0:
            print('git pull --rebase failed — resolve by hand, not force-pushing:',
                  pull.stderr.strip(), sep='\n  ')
            record('pull_failed', started)
            return 5

    if not args.skip_export:
        extra = []
        if args.section:
            extra += ['--section', args.section]
        if args.limit:
            extra += ['--limit', str(args.limit)]
        if args.ignore_pause:
            extra.append('--ignore-pause')      # or the export refuses and the run dies here
        rc = stage_step('export.py', extra)
        if rc != 0:
            print(f'export failed (rc={rc}) — nothing written', file=sys.stderr)
            record(f'export_rc_{rc}', started)
            return rc

    rc = stage_step('convert.py')
    if rc != 0:
        print('conversion gate failed — nothing written to the vault', file=sys.stderr)
        record('convert_failed', started)
        return rc

    state = json.loads(STATE_FILE.read_text()) if STATE_FILE.exists() else {}
    fresh = collect(OUT)
    changed, added = [], []

    for rel, meta in fresh.items():
        dest = SPECS / rel
        old = state.get(rel)
        if old and old.get('md_sha') == meta['md_sha'] and dest.exists():
            continue
        dest.parent.mkdir(parents=True, exist_ok=True)
        src = OUT / rel
        if src.stat().st_size > MAX_FILE_BYTES:
            print(f'  ! skipping oversized {rel} ({src.stat().st_size} bytes)')
            continue
        (added if old is None else changed).append(rel)
        dest.write_text(src.read_text())
        for img in meta['images']:                      # content-addressed, so a
            s = (OUT / rel).parent / img                # matching name means the
            d = dest.parent / img                       # same bytes — copy once
            if s.exists() and not d.exists():
                d.parent.mkdir(parents=True, exist_ok=True)
                d.write_bytes(s.read_bytes())
        state[rel] = {**meta, 'synced_at': started}

    # a page that vanished from the notebook is only removed when the export ran
    # in full: a partial run must never look like a deletion
    # Deletions are applied only after a run that saw every page and exported it.
    # A page that failed to render must never be mistaken for a deleted one.
    index = {}
    idx_file = ROOT / 'data' / 'staging' / 'index.json'
    if idx_file.exists():
        index = json.loads(idx_file.read_text())
    complete = (isinstance(index, dict)
                and not index.get('partial')
                and len(index.get('exported', [])) == len(index.get('seen', [1])))
    removed = []
    if complete and not args.section and not args.limit and not args.skip_export:
        for rel in [r for r in state if r not in fresh]:
            note = SPECS / rel
            if note.exists():
                note.unlink()
            removed.append(rel)
            state.pop(rel, None)

    if not complete and not args.section and not args.limit:
        print(f'  ! partial export ({len(index.get("exported", []))}/'
              f'{len(index.get("seen", []))}) — deletions skipped this run')

    # content-addressed image names orphan a file whenever an edit drops the
    # image, so sweep every image against what the notes reference. NFC on both
    # sides: macOS hands paths back in NFD and the mismatch would nuke everything.
    def nfc(p):
        return unicodedata.normalize('NFC', p.as_posix())
    referenced = set()
    for rel, meta in state.items():
        base = (SPECS / rel).parent
        referenced |= {nfc(base / img) for img in meta.get('images', [])}
    orphans = [f for f in SPECS.rglob('images/*/*')
               if f.is_file() and nfc(f) not in referenced]
    for f in orphans:
        f.unlink()
        if not any(f.parent.iterdir()):
            f.parent.rmdir()
    if orphans:
        print(f'  swept {len(orphans)} orphaned images')

    print(f'\nadded {len(added)} · changed {len(changed)} · removed {len(removed)}')
    for rel in (added + changed + removed)[:20]:
        print('   ', rel)

    if args.dry_run:
        print('dry run — vault files updated, no commit')
        return 0

    # the commit decision comes from git, not our bookkeeping: a previous run
    # interrupted between write and commit left real changes with a state file
    # that already claims them — those must still land in a commit
    rel_specs = SPECS.relative_to(VAULT).as_posix()
    STATE_FILE.write_text(json.dumps(state, indent=2, ensure_ascii=False, sort_keys=True))
    dirty = run(['git', 'status', '--porcelain', '--', rel_specs], VAULT).stdout.strip()
    n = len(added) + len(changed)
    if dirty:
        run(['git', 'add', '--', rel_specs], VAULT)
        msg = (f'docs(specs): sync {n} page{"s" if n != 1 else ""} from OneNote'
               + (f' ({len(removed)} removed)' if removed else '')
               if n or removed else
               'docs(specs): sync maintenance (orphaned images / interrupted run)')
        run(['git', 'commit', '-q', '-m', msg], VAULT)
    else:
        # an earlier run whose push was rejected left its commit sitting here,
        # and this bookkeeping already calls those pages synced — nothing will
        # ever produce them again, so an otherwise idle run carries them out
        msg = 'carrying out a commit an earlier run could not push'
    ahead = run(['git', 'rev-list', '--count', '@{u}..HEAD'],
                VAULT, check=False).stdout.strip()
    if not dirty and ahead in ('', '0'):
        record('no_changes', started)
        print('nothing to commit')
        return 0

    err = '' if args.no_push else push(VAULT)
    # after the push, never before: a retry rebases and HEAD moves
    head = run(['git', 'rev-parse', '--short', 'HEAD'], VAULT).stdout.strip()
    if err:
        print('commit made but push failed:', err)
    print(f'committed {head}: {msg}')
    # a commit that never left the container is not a synced vault. Recording it
    # as success is how five changed pages sat here under a green pill.
    record('push_failed' if err else 'committed', started, commit=head,
           added=len(added), changed=len(changed), removed=len(removed))
    return 6 if err else 0


if __name__ == '__main__':
    sys.exit(main())
