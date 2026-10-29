"""The runs history: clearing it whole or row by row, and the per-run log archive.

Nothing here starts a real subprocess — spawn() is exercised with a stand-in, so
the id it mints and the log file it opens are checked without a run ever touching
the client's SharePoint or the vault.
"""
import json
import threading
import time
from datetime import datetime, timedelta, timezone

import pytest
from conftest import Running


def iso_ago(**delta):
    return (datetime.now(timezone.utc) - timedelta(**delta)).isoformat(timespec='seconds')


def row(run_id=None, **extra):
    r = {'started': iso_ago(hours=2), 'outcome': 'no_changes', **extra}
    if run_id:
        r['run_id'] = run_id
    return json.dumps(r) + '\n'


class FinishedProc:
    """A subprocess that has already exited."""

    def poll(self):
        return 0

    def wait(self):
        return 0


class NoThread:
    def __init__(self, *a, **kw):
        pass

    def start(self):
        pass


@pytest.fixture
def spawner(admin, monkeypatch):
    """spawn() with no real subprocess and no reaping thread; yields what it asked for."""
    calls = []

    def popen(argv, **kw):
        kw['stdout'].close()        # a real Popen dups the fd, the parent's copy goes
        calls.append({'argv': argv, 'env': kw['env']})
        return FinishedProc()

    monkeypatch.setattr(admin.subprocess, 'Popen', popen)
    monkeypatch.setattr(admin.threading, 'Thread', NoThread)
    return calls


# ------------------------------------------------------------- the log archive

def test_a_run_gets_an_id_and_a_log_file_named_after_it(admin, spawner):
    assert admin.spawn(['sync.py'], 'sync') is True
    run_id = spawner[0]['env']['RUN_ID']
    # the runner is handed the id, so the row it writes and this file agree
    assert (admin.RUN_LOGS / f'{run_id}.log').exists()


def test_a_runs_log_survives_the_next_run_and_is_retrievable_by_id(client, admin, auth,
                                                                  spawner):
    admin.spawn(['sync.py'], 'sync')
    first = spawner[0]['env']['RUN_ID']
    (admin.RUN_LOGS / f'{first}.log').write_text('first run output')

    admin.spawn(['sync.py'], 'sync')
    second = spawner[1]['env']['RUN_ID']
    assert second != first
    (admin.RUN_LOGS / f'{second}.log').write_text('second run output')

    # the whole point: the earlier log is still there afterwards, not overwritten
    assert client.get('/log', query_string={'id': first},
                      headers=auth).data.decode() == 'first run output'
    # and with no id asked for, the newest run's log
    assert client.get('/log', headers=auth).data.decode() == 'second run output'


def test_the_log_archive_is_capped(admin, spawner, monkeypatch):
    monkeypatch.setattr(admin, 'LOG_KEEP', 2)
    admin.RUN_LOGS.mkdir()
    for old in ('20260801T000000-000000', '20260802T000000-000000'):
        (admin.RUN_LOGS / f'{old}.log').write_text('old output')

    admin.spawn(['sync.py'], 'sync')

    kept = sorted(p.stem for p in admin.RUN_LOGS.glob('*.log'))
    assert len(kept) == 2
    assert '20260801T000000-000000' not in kept          # the oldest went
    assert kept[-1] == spawner[0]['env']['RUN_ID']       # the new one never does


def test_a_log_id_cannot_reach_outside_the_archive(client, admin, auth, tmp_path):
    (tmp_path / 'secret.log').write_text('the saved session')
    admin.RUN_LOGS.mkdir()
    for probe in ('../secret', '../../etc/passwd', 'no-such-run'):
        r = client.get('/log', query_string={'id': probe}, headers=auth)
        assert r.status_code == 404, probe
        assert 'saved session' not in r.data.decode(), probe


def test_a_run_records_the_id_the_admin_minted(syncer, monkeypatch):
    monkeypatch.setenv('RUN_ID', '20260803T090000-000001')
    syncer.record('no_changes', iso_ago(hours=1))
    written = json.loads(syncer.RUNS_LOG.read_text().splitlines()[-1])
    assert written['run_id'] == '20260803T090000-000001'


def test_a_run_started_from_a_shell_carries_no_id(syncer, monkeypatch):
    monkeypatch.delenv('RUN_ID', raising=False)
    syncer.record('no_changes', iso_ago(hours=1))
    assert 'run_id' not in json.loads(syncer.RUNS_LOG.read_text().splitlines()[-1])


# ------------------------------------------------------------ clearing history

def test_clearing_the_history_removes_the_rows_and_their_logs(client, admin, auth):
    admin.RUNS.write_text(row('a1') + row('a2'))
    admin.LAST_RUN.write_text(json.dumps({'started': iso_ago(hours=2),
                                          'outcome': 'no_changes'}))
    admin.RUN_LOGS.mkdir()
    (admin.RUN_LOGS / 'a1.log').write_text('output')
    (admin.RUN_LOGS / 'a2.log').write_text('output')

    s = client.post('/runs/clear', headers=auth).get_json()

    assert s['runs'] == []
    assert not admin.RUNS.exists()
    assert list(admin.RUN_LOGS.glob('*.log')) == []      # and no orphaned logs behind
    # the health line derives from those rows, so it now says so honestly instead
    # of holding on to the state of a run that no longer exists
    assert s['last_run'] is None
    assert s['health'] == {'text': 'stale · never ran', 'kind': 'bad'}


def test_rewriting_the_history_is_refused_while_a_run_is_in_flight(client, admin, auth,
                                                                  monkeypatch):
    monkeypatch.setattr(admin, 'proc', Running())
    admin.RUNS.write_text(row('a1'))

    s = client.post('/runs/clear', headers=auth).get_json()
    assert 'wait for it to finish' in s['notice']['text']
    assert len(s['runs']) == 1 and admin.RUNS.exists()

    # a run appending its row races a rewrite the same way whether one row is
    # dropped or all of them, so the guard covers both
    s = client.post('/runs/delete', data={'id': 'a1'}, headers=auth).get_json()
    assert s['notice'] is not None and len(s['runs']) == 1


def test_deleting_one_run_leaves_the_others_and_drops_its_log(client, admin, auth):
    admin.RUNS.write_text(row('a1') + row('a2'))
    admin.RUN_LOGS.mkdir()
    (admin.RUN_LOGS / 'a1.log').write_text('first output')
    (admin.RUN_LOGS / 'a2.log').write_text('second output')

    s = client.post('/runs/delete', data={'id': 'a1'}, headers=auth).get_json()

    assert [r['run_id'] for r in s['runs']] == ['a2']
    assert not (admin.RUN_LOGS / 'a1.log').exists()
    assert (admin.RUN_LOGS / 'a2.log').read_text() == 'second output'
    # replaced whole: every remaining line is complete json, and the scratch file
    # it was built in is gone
    assert [json.loads(l)['run_id'] for l in admin.RUNS.read_text().splitlines()] == ['a2']
    assert not list(admin.DATA.glob('*.tmp'))


def test_a_row_from_before_ids_stays_in_the_table_and_deletes_by_timestamp(client, admin,
                                                                          auth):
    started = iso_ago(hours=5)
    admin.RUNS.write_text(json.dumps({'started': started, 'outcome': 'no_changes'}) + '\n'
                          + row('a2'))

    s = client.get('/status.json', headers=auth).get_json()
    assert [r.get('run_id') for r in s['runs']] == ['a2', None]      # it is still listed

    s = client.post('/runs/delete', data={'started': started}, headers=auth).get_json()
    assert [r.get('run_id') for r in s['runs']] == ['a2']


def test_deleting_a_row_that_is_already_gone_changes_nothing(client, admin, auth):
    admin.RUNS.write_text(row('a2'))
    before = admin.RUNS.read_text()
    s = client.post('/runs/delete', data={'id': 'a1'}, headers=auth).get_json()
    assert len(s['runs']) == 1
    assert admin.RUNS.read_text() == before


def test_two_deletes_at_once_do_not_lose_each_others_rows(client, admin, auth, monkeypatch):
    """The server is threaded, so two people can be deleting rows at the same
    moment. Each one reads the history and writes it back, so without a lock the
    second write puts back the row the first had just removed.
    """
    admin.RUNS.write_text(row('a1') + row('a2') + row('a3'))
    real_read = admin.read_runs

    def slow_read():
        rows = real_read()
        time.sleep(0.05)        # widen the read-then-write window to the naked eye
        return rows

    monkeypatch.setattr(admin, 'read_runs', slow_read)

    def delete(run_id):
        admin.app.test_client().post('/runs/delete', data={'id': run_id}, headers=auth)

    threads = [threading.Thread(target=delete, args=(r,)) for r in ('a1', 'a3')]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    # both rows are gone and the untouched one survived: neither writer wrote back
    # what the other had already removed, and each built its own scratch file
    assert [json.loads(l)['run_id'] for l in admin.RUNS.read_text().splitlines()] == ['a2']
    assert not list(admin.DATA.glob('*.tmp'))


def test_a_run_cannot_start_between_the_refusal_check_and_the_edit(admin, auth):
    """Checking whether a run is in flight and editing have to be one step. A run
    that starts in the gap has already opened its log, and the clear would then
    unlink it and leave the run writing to a file nobody can read.
    """
    admin.RUNS.write_text(row('a1'))
    cleared = threading.Event()

    def clear():
        admin.app.test_client().post('/runs/clear', headers=auth)
        cleared.set()

    with admin.proc_lock:               # exactly what starting a run holds
        threading.Thread(target=clear, daemon=True).start()
        assert not cleared.wait(0.2)    # so the clear cannot slip past the check
        assert admin.RUNS.exists()

    assert cleared.wait(2)
    assert not admin.RUNS.exists()


def test_a_rewrite_that_dies_halfway_keeps_the_old_history(admin, monkeypatch):
    admin.RUNS.write_text(row('a1') + row('a2'))
    before = admin.RUNS.read_text()

    def full_disk(self, target):
        raise OSError('no space left on device')

    monkeypatch.setattr(admin.Path, 'replace', full_disk)
    with pytest.raises(OSError):
        admin.write_runs([])
    # the rows are all still there: nothing was ever truncated in place
    assert admin.RUNS.read_text() == before
