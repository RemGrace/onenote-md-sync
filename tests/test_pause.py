"""The pause sentinel stops the schedule, not the person at the keyboard.

The one exception is a smart lockout: it writes the same file, and running
through it — a sign-in above all — only deepens the lockout.
"""
import json
import subprocess
import sys

from pause import blocks, is_lockout, read_pause

MANUAL = json.dumps({'reason': 'manual pause', 'at': '2026-08-03T06:00:00+00:00'})
LOCKOUT = json.dumps({'reason': 'AADSTS50053 smart lockout',
                      'at': '2026-08-03T06:00:00+00:00'})


# ------------------------------------------------------------- the two flavours

def test_only_microsoft_pauses_count_as_a_lockout():
    assert is_lockout(json.loads(LOCKOUT))
    assert not is_lockout(json.loads(MANUAL))
    assert not is_lockout({'reason': 'stop it, debugging'})
    assert not is_lockout(None)


def test_a_hand_started_run_goes_through_a_manual_pause_only():
    assert blocks(None) is False
    assert blocks(None, by_hand=True) is False
    assert blocks(json.loads(MANUAL)) is True
    assert blocks(json.loads(MANUAL), by_hand=True) is False
    assert blocks(json.loads(LOCKOUT)) is True
    assert blocks(json.loads(LOCKOUT), by_hand=True) is True


def test_a_file_written_by_hand_still_pauses(tmp_path):
    sentinel = tmp_path / 'PAUSED'
    assert read_pause(sentinel) is None
    sentinel.write_text('stop it, debugging')
    assert read_pause(sentinel) == {'reason': 'stop it, debugging'}
    sentinel.write_text('')
    assert read_pause(sentinel) == {'reason': 'paused'}
    sentinel.write_text(MANUAL)
    assert read_pause(sentinel)['reason'] == 'manual pause'


# ------------------------------------------------------------------- signing in

def test_a_re_login_gets_through_a_paused_schedule(exporter, monkeypatch):
    exporter.PAUSED.write_text(MANUAL)
    monkeypatch.setattr(sys, 'argv', ['export.py', '--login-only'])
    assert exporter.main() == 2


def test_a_re_login_still_stops_on_a_lockout(exporter, monkeypatch):
    exporter.PAUSED.write_text(LOCKOUT)
    monkeypatch.setattr(sys, 'argv', ['export.py', '--login-only'])
    assert exporter.main() == 7


def test_a_scheduled_export_still_stops_on_a_paused_schedule(exporter, monkeypatch):
    exporter.PAUSED.write_text(MANUAL)
    monkeypatch.setattr(sys, 'argv', ['export.py'])
    assert exporter.main() == 7


def test_an_export_asked_for_by_hand_gets_through_a_paused_schedule(exporter, monkeypatch):
    exporter.PAUSED.write_text(MANUAL)
    monkeypatch.setattr(sys, 'argv', ['export.py', '--ignore-pause'])
    assert exporter.main() == 2


# ----------------------------------------------------------------- a whole run

def test_a_sync_asked_for_by_hand_runs_while_paused(syncer, monkeypatch):
    (syncer.ROOT / 'data' / 'PAUSED').write_text(MANUAL)
    monkeypatch.setattr(sys, 'argv', ['sync.py', '--ignore-pause'])
    assert syncer.main() == 3
    assert not syncer.LAST_RUN.exists()          # nothing recorded as skipped


def test_a_sync_asked_for_by_hand_still_stops_on_a_lockout(syncer, monkeypatch):
    (syncer.ROOT / 'data' / 'PAUSED').write_text(LOCKOUT)
    monkeypatch.setattr(sys, 'argv', ['sync.py', '--ignore-pause'])
    assert syncer.main() == 7
    assert json.loads(syncer.LAST_RUN.read_text())['outcome'] == 'paused'


def test_a_scheduled_sync_stops_on_a_paused_schedule(syncer, monkeypatch):
    (syncer.ROOT / 'data' / 'PAUSED').write_text(MANUAL)
    monkeypatch.setattr(sys, 'argv', ['sync.py'])
    assert syncer.main() == 7


def test_a_sync_asked_for_by_hand_carries_the_override_into_the_export(syncer, monkeypatch):
    # without the pass-through the run would die at the export's own pause guard
    (syncer.ROOT / 'data' / 'PAUSED').write_text(MANUAL)
    syncer.SPECS.mkdir(parents=True)
    monkeypatch.setattr(syncer, 'run', lambda *a, **k: subprocess.CompletedProcess(
        args=[], returncode=0, stdout='', stderr=''))
    steps = []
    monkeypatch.setattr(syncer, 'stage_step',
                        lambda script, extra=(): steps.append((script, list(extra))) or 9)
    monkeypatch.setattr(sys, 'argv', ['sync.py', '--ignore-pause'])
    assert syncer.main() == 9
    assert steps == [('export.py', ['--ignore-pause'])]
