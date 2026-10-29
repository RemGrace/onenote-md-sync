"""Fixtures: the admin app with every data path pointed at a tmp dir.

The real data/ directory holds a live session and must never be touched by
tests; every module-level Path constant is monkeypatched per test.
"""
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import app as admin_app
import export as export_script
import sync as sync_script

_PATH_NAMES = ['PAUSED', 'PENDING', 'OTP_ANSWER', 'STATE', 'RUNS', 'LAST_RUN',
               'RUN_LOGS', 'SCHEDULE', 'SCHEDULE_SETTINGS', 'ATTEMPTS', 'ACCOUNT',
               'LOGIN_RESULT']


class Running:
    """Stands in for a live sync/login subprocess."""

    def poll(self):
        return None


@pytest.fixture
def admin(tmp_path, monkeypatch):
    monkeypatch.setattr(admin_app, 'DATA', tmp_path)
    for name in _PATH_NAMES:
        monkeypatch.setattr(admin_app, name, tmp_path / getattr(admin_app, name).name)
    monkeypatch.setattr(admin_app, 'TOKEN', 'test-token')
    monkeypatch.setattr(admin_app, 'proc', None)
    # the account falls back to the env var, so a developer's own shell must not
    # decide what these tests see
    monkeypatch.delenv('MS_EMAIL', raising=False)
    monkeypatch.setattr(admin_app, 'login_flow',
                        {'phase': 'idle', 'reason': None, 'codes': 0})
    # the probe calls the real SharePoint — no test may ever leave the box
    monkeypatch.setattr(admin_app, 'probe_session',
                        lambda: {'ok': False, 'reason': 'no storage-state.json'})
    # the probe result is cached for a minute in module state; left alone, the
    # first test's session would be handed to every later one
    monkeypatch.setattr(admin_app, '_probe_cache', (0.0, None))
    admin_app.FAILURES.clear()
    return admin_app


@pytest.fixture
def client(admin):
    admin.app.config['TESTING'] = True
    return admin.app.test_client()


@pytest.fixture
def bundle(admin, tmp_path, monkeypatch):
    """A stand-in for frontend/dist, so the python gate never needs a node build.

    The real bundle's asset names are content hashes; only the shape matters here
    — an index.html and something under assets/.
    """
    dist = tmp_path / 'dist'
    (dist / 'assets').mkdir(parents=True)
    (dist / 'index.html').write_text('<!doctype html><div id="root"></div>')
    (dist / 'assets' / 'index-abc123.js').write_text('console.log(1)')
    monkeypatch.setattr(admin, 'DIST', dist)
    monkeypatch.setattr(admin.app, 'static_folder', dist / 'assets')
    return dist


@pytest.fixture
def auth():
    return {'X-Admin-Token': 'test-token'}


@pytest.fixture
def exporter(tmp_path, monkeypatch):
    """export.py with its files in tmp and no session to reuse.

    main() then stops at the 'no session' exit (2) instead of ever reaching
    Chromium or SharePoint — so reaching 2 is the proof it got past the pause.
    """
    for name in ['PAUSED', 'STATE', 'STAGING', 'ATTEMPTS', 'PENDING', 'OTP_ANSWER',
                 'ACCOUNT', 'LOGIN_RESULT']:
        monkeypatch.setattr(export_script, name,
                            tmp_path / getattr(export_script, name).name)
    monkeypatch.delenv('MS_EMAIL', raising=False)
    return export_script


@pytest.fixture
def syncer(tmp_path, monkeypatch):
    """sync.py with its records in tmp and a vault path that does not exist.

    main() then stops at the missing specs tree (3) instead of ever running git
    against the real vault — so reaching 3 is the proof it got past the pause.
    """
    (tmp_path / 'data').mkdir()
    monkeypatch.setattr(sync_script, 'ROOT', tmp_path)
    monkeypatch.setattr(sync_script, 'SPECS', tmp_path / 'nowhere' / 'specs')
    monkeypatch.setattr(sync_script, 'LAST_RUN', tmp_path / 'last_run.json')
    monkeypatch.setattr(sync_script, 'RUNS_LOG', tmp_path / 'runs.jsonl')
    return sync_script
