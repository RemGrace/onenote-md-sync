"""Progress is read off what a run prints, so those printed phrases are an input
to app.py as much as any function signature is.

The first test pins them in the scripts that produce them. It exists because the
export and the conversion drive the client's SharePoint and vault: nobody is
going to instrument them, so the wording is the contract, and a reworded print()
would otherwise leave the run page silently empty with the whole suite green.
"""
import json
from pathlib import Path

import pytest
from conftest import Running

ROOT = Path(__file__).resolve().parent.parent

# Every phrase the parser matches on, next to the file that prints it. Verbatim
# source text, so a reformatted f-string trips this too.
PINNED = {
    'sync.py': [
        "print(f'--- {script} {\" \".join(extra)}')",
        # the header only names a phase because these two call sites pass the file
        # name: renaming either script empties every run page, and the template
        # above would still be there to keep this green
        "rc = stage_step('export.py', extra)",
        "rc = stage_step('convert.py')",
        "print(f'\\nadded {len(added)} · changed {len(changed)} · removed {len(removed)}')",
        "print(f'  ! partial export ({len(index.get(\"exported\", []))}/'",
        "f'{len(index.get(\"seen\", []))}) — deletions skipped this run')",
        "print(f'committed {head}: {msg}')",
        "print('nothing to commit')",
        "print('dry run — vault files updated, no commit')",
    ],
    'export.py': [
        "print(f'rails sections ({len(sections)}): {[s[\"path\"] for s in sections]}')",
        "print(f'    {len(pages)} pages')",
        "print(f'    ok {section_path}/{name}.html  '",
        "print(f'  skip {path} — unchanged upstream ({len(metas)} staged pages)')",
        "print(f'\\n{len(exported)}/{len(seen)} pages staged in {STAGING}'",
    ],
    'convert.py': [
        "print(f'{flag} {ratio:6.1%} lost={lost:5d}/{total:6d} imgs={len(refs)}  {rel}')",
        "print(f'\\n{done} converted, worst coverage {worst:.1%}')",
        "flag = 'ok  ' if ratio >= args.min_coverage else 'LOSS'",
    ],
}


@pytest.mark.parametrize('script', sorted(PINNED))
def test_the_lines_progress_is_counted_from_still_read_that_way(script):
    source = (ROOT / script).read_text()
    for phrase in PINNED[script]:
        assert phrase in source, (
            f'{script} no longer prints this, and the admin counts it:\n  {phrase}\n'
            'Update the patterns in app.py in the same change, or the run page '
            'goes blank with nothing failing.')


# ------------------------------------------------------------- the log parser

MID_EXPORT = """--- export.py --ignore-pause
REST sections (6): ['Client', 'ERP']
rails sections (6): ['Client', 'ERP']
  section Client
    18 pages
    ok Client/Grille tarifaires.html  (42KB, tables=3, imgs=1)
    ok Client/Contacts.html  (11KB, tables=0, imgs=0)
"""

CRAWLED_EVERY_SECTION = """--- export.py
rails sections (2): ['Client', 'ERP']
  section Client
    3 pages
    ok Client/One.html  (1KB, tables=0, imgs=0)
    ok Client/Two.html  (1KB, tables=0, imgs=0)
  skip ERP — unchanged upstream (2 staged pages)
"""

FULL_RUN = """--- export.py
rails sections (2): ['Client', 'ERP']
  section Client
    2 pages
    ok Client/One.html  (1KB, tables=0, imgs=0)
    ok Client/Two.html  (1KB, tables=0, imgs=0)
  skip ERP — unchanged upstream (2 staged pages)

4/4 pages staged in /app/data/staging
--- convert.py
ok  100.0% lost=    0/  1200 imgs=1  Client/One.html
ok   99.8% lost=    2/  1200 imgs=0  Client/Two.html
LOSS  98.0% lost=   20/  1000 imgs=0  ERP/Three.html
ok  100.0% lost=    0/   900 imgs=0  ERP/Four.html

4 converted, worst coverage 98.0%

added 3 · changed 1 · removed 0
    Client/One.md
committed a7f3c21: docs(specs): sync 4 pages from OneNote
"""

PARTIAL_RUN = FULL_RUN.replace(
    'added 3 · changed 1 · removed 0',
    '  ! partial export (116/118) — deletions skipped this run\n'
    'added 3 · changed 1 · removed 0')


def test_a_sign_in_run_has_no_progress_at_all(admin):
    """It prints no phase header, writes no history row, and gets no run page."""
    assert admin.run_progress('  login ok\nsession saved\n') is None


def test_no_percentage_until_every_section_has_been_crawled(admin):
    p = admin.run_progress(MID_EXPORT, live=True)
    assert p['phase'] == 'export'
    assert p['done'] == 2
    # one of six sections crawled: the denominator does not exist yet, and a
    # guessed bar that jumps backwards is worse than none
    assert p['total'] is None and p['percent'] is None
    assert p['indeterminate'] is True
    assert '6 sections found' in p['counters'][0]


def test_the_percentage_appears_once_the_page_count_is_known(admin):
    p = admin.run_progress(CRAWLED_EVERY_SECTION, live=True)
    # 3 crawled + 2 standing in from the skipped section
    assert (p['done'], p['total'], p['percent']) == (4, 5, 80)
    assert p['indeterminate'] is False
    assert p['counters'][0] == '2 sections found · 1 skipped as unchanged upstream'
    assert p['counters'][1] == '4 pages exported · 1 remaining'


def test_a_finished_run_reports_every_phase_the_counters_and_the_commit(admin):
    p = admin.run_progress(FULL_RUN)
    assert p['ended'] is True
    assert p['headline'] == 'Finished'
    assert [s['state'] for s in p['phases']] == ['done'] * 4
    assert [s['label'] for s in p['phases']] == ['export', 'convert', 'vault delta',
                                                 'commit']
    assert p['done'] == 4 and p['total'] == 4 and p['percent'] == 100
    assert p['counters'][-1] == '4 pages converted'
    assert p['delta'] == '3 added · 1 changed · 0 removed'
    assert p['commit'] == 'a7f3c21'
    assert p['note'] is None


def test_a_suppressed_removal_is_explained_not_left_looking_like_a_bug(admin):
    p = admin.run_progress(PARTIAL_RUN)
    assert p['note'] is not None
    # the numbers as the run reported them, and why nothing was deleted
    assert '118 pages' in p['note'] and '116' in p['note']
    assert 'deleted' in p['note']


def test_a_run_that_died_mid_phase_does_not_read_as_still_working(admin):
    p = admin.run_progress(MID_EXPORT, live=False)
    assert p['headline'] == 'Stopped before finishing'
    assert [s['state'] for s in p['phases']] == ['stopped', 'pending', 'pending',
                                                 'pending']


# ---------------------------------------------------------------- the endpoint

def write_log(admin, run_id, text):
    admin.RUN_LOGS.mkdir(parents=True, exist_ok=True)
    (admin.RUN_LOGS / f'{run_id}.log').write_text(text)


def test_a_run_page_serves_the_progress_the_row_and_the_log(client, admin, auth):
    write_log(admin, 'r1', FULL_RUN)
    admin.RUNS.write_text(json.dumps({'started': '2026-08-03T08:30:00+00:00',
                                      'outcome': 'committed', 'run_id': 'r1',
                                      'commit': 'a7f3c21', 'added': 3, 'changed': 1,
                                      'removed': 0}) + '\n')

    body = client.get('/run.json', query_string={'id': 'r1'}, headers=auth).get_json()

    assert body['live'] is False
    assert body['progress']['commit'] == 'a7f3c21'
    assert body['run']['outcome_text'] == 'synced 4 pages'
    assert '--- export.py' in body['log']


def test_a_run_still_in_flight_reads_as_live(client, admin, auth, monkeypatch):
    write_log(admin, 'r1', MID_EXPORT)
    monkeypatch.setattr(admin, 'proc', Running())

    body = client.get('/run.json', query_string={'id': 'r1'}, headers=auth).get_json()
    assert body['live'] is True
    assert body['progress']['headline'] == 'Scanning OneNote'
    # and an older run is not dragged live by the one that is running now
    write_log(admin, 'r0', MID_EXPORT)          # sorts before r1
    body = client.get('/run.json', query_string={'id': 'r0'}, headers=auth).get_json()
    assert body['live'] is False


def test_a_run_that_ended_before_the_export_still_serves_its_log(client, admin, auth):
    """A pause or a vault conflict ends a run before the export prints anything,
    and the row in the history links to this page. The log is the only place that
    says what to do about it, so the page has to carry it rather than 404."""
    write_log(admin, 'r1', 'git pull --rebase failed — resolve by hand\n  CONFLICT\n')
    admin.RUNS.write_text(json.dumps({'started': '2026-08-03T08:30:00+00:00',
                                      'outcome': 'pull_failed', 'run_id': 'r1'}) + '\n')

    r = client.get('/run.json', query_string={'id': 'r1'}, headers=auth)

    assert r.status_code == 200
    body = r.get_json()
    assert body['progress'] is None          # no phase ever started
    assert body['run']['outcome_text'] == 'vault conflict — fix by hand'
    assert 'CONFLICT' in body['log']


def test_a_login_run_has_no_run_page(client, admin, auth):
    write_log(admin, 'r1', '  login ok\nsession saved\n')
    r = client.get('/run.json', query_string={'id': 'r1'}, headers=auth)
    assert r.status_code == 404
    assert 'session card' in r.get_json()['error']


def test_a_run_page_id_cannot_reach_outside_the_archive(client, admin, auth, tmp_path):
    (tmp_path / 'secret.log').write_text('--- export.py\nthe saved session\n')
    admin.RUN_LOGS.mkdir()
    for probe in ('../secret', '../../etc/passwd', 'no-such-run'):
        r = client.get('/run.json', query_string={'id': probe}, headers=auth)
        assert r.status_code == 404, probe
        assert 'saved session' not in r.data.decode(), probe


def test_the_dashboard_gets_one_line_pointing_at_the_running_runs_page(client, admin,
                                                                      auth, monkeypatch):
    write_log(admin, 'r1', CRAWLED_EVERY_SECTION)
    monkeypatch.setattr(admin, 'proc', Running())

    s = client.get('/status.json', headers=auth).get_json()
    assert s['run_progress'] == {'run_id': 'r1', 'text': 'running — export, 4/5'}


def test_nothing_running_means_no_progress_line(client, admin, auth):
    write_log(admin, 'r1', FULL_RUN)
    assert client.get('/status.json', headers=auth).get_json()['run_progress'] is None


def test_a_run_page_url_is_bookmarkable_and_survives_a_reload(client, admin, auth,
                                                             bundle):
    """A reload of /runs/<id> has to answer with the shell, and an expired session
    there has to land on the sign-in form rather than a bare 403."""
    assert client.get('/runs/r1', headers=auth).status_code == 200
    assert b'id="root"' in client.get('/runs/r1', headers=auth).data
    assert 'admin token' in client.get('/runs/r1').data.decode()
    # a typo is still a typo, not the shell
    assert client.get('/runs/r1/extra', headers=auth).status_code == 404
