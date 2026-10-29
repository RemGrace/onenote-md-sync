import json
import time
from datetime import datetime, timedelta, timezone

from conftest import Running


def iso_ago(**delta):
    return (datetime.now(timezone.utc) - timedelta(**delta)).isoformat(timespec='seconds')


FRESH_RUN = {'outcome': 'committed', 'commit': 'abc1234',
             'added': 1, 'changed': 0, 'removed': 0}


# --------------------------------------------------------------- code prompt

def test_code_without_waiting_login_answers_a_notice(client, auth):
    r = client.post('/code', data={'code': '123456'}, headers=auth)
    assert r.status_code == 200
    assert 'No login is waiting for a code' in r.get_json()['notice']['text']


def test_code_with_waiting_login_writes_answer(client, admin, auth, monkeypatch):
    monkeypatch.setattr(admin, 'proc', Running())
    admin.PENDING.write_text(json.dumps({'hint': 'code sent', 'deadline_epoch': 0}))
    r = client.post('/code', data={'code': ' 42 '}, headers=auth)
    assert r.get_json()['notice'] is None
    assert json.loads(admin.OTP_ANSWER.read_text()) == {'code': '42'}


def test_code_for_a_dead_runner_is_told_so_instead_of_vanishing(client, admin, auth):
    admin.PENDING.write_text(json.dumps({'hint': 'code sent', 'deadline_epoch': 0}))
    r = client.post('/code', data={'code': '123456'}, headers=auth)
    assert r.get_json()['notice'] is not None
    assert not admin.OTP_ANSWER.exists()


def test_empty_code_is_a_noop(client, admin, auth):
    admin.PENDING.write_text(json.dumps({'hint': 'code sent', 'deadline_epoch': 0}))
    r = client.post('/code', data={'code': '  '}, headers=auth)
    assert r.get_json()['notice'] is None      # nothing happened, nothing to say
    assert not admin.OTP_ANSWER.exists()


# ------------------------------------------------------------------ the bundle

def test_the_shell_is_served_to_a_signed_in_session(client, auth, bundle):
    assert '<div id="root">' in client.get('/', headers=auth).data.decode()
    assert client.get('/assets/index-abc123.js', headers=auth).status_code == 200


def test_an_anonymous_visitor_gets_the_sign_in_form_never_the_bundle(client, bundle):
    page = client.get('/').data.decode()
    assert 'Sign in' in page and 'name="token"' in page
    assert 'id="root"' not in page
    # the hashed assets are the app's own static folder, so they sit behind the
    # same before_request gate and nothing of the UI leaks to a stranger
    assert client.get('/assets/index-abc123.js').status_code == 403


def test_a_client_side_route_reloaded_in_place_answers_the_shell(client, auth, bundle):
    for path in ('/runs', '/settings'):
        assert '<div id="root">' in client.get(path, headers=auth).data.decode(), path
    assert client.get('/nope', headers=auth).status_code == 404
    # a bookmarked page whose session aged out lands on sign-in, not a bare 403
    assert 'Sign in' in client.get('/runs').data.decode()


def test_a_missing_bundle_says_how_to_build_it(client, admin, auth, tmp_path, monkeypatch):
    monkeypatch.setattr(admin, 'DIST', tmp_path / 'never-built')
    r = client.get('/', headers=auth)
    assert r.status_code == 503
    assert 'npm --prefix frontend run build' in r.data.decode()


# ------------------------------------------------------------ human statuses

def test_run_outcomes_render_as_words(admin):
    assert admin.human_outcome({'outcome': 'committed', 'added': 2, 'changed': 1}) \
        == ('synced 3 pages', 'ok')
    assert admin.human_outcome({'outcome': 'committed', 'added': 1, 'changed': 0})[0] \
        == 'synced 1 page'
    assert admin.human_outcome({'outcome': 'no_changes'}) == ('no changes upstream', 'ok')
    assert admin.human_outcome({'outcome': 'export_rc_2'}) \
        == ('session dead — re-login needed', 'bad')
    assert admin.human_outcome({'outcome': 'export_rc_3'}) == ('failed — see log', 'bad')
    assert admin.human_outcome({'outcome': 'convert_failed'})[0] \
        == 'conversion check failed — see log'
    assert admin.human_outcome({'outcome': 'pull_failed'})[0] == 'vault conflict — fix by hand'
    assert admin.human_outcome({'outcome': 'push_failed', 'added': 0, 'changed': 5}) \
        == ('committed but not pushed — GitHub is behind', 'bad')
    assert admin.human_outcome({'outcome': 'paused'}) == ('paused', 'mut')
    assert admin.human_outcome(None) == ('never ran', 'mut')


def test_probe_failures_render_as_words(admin):
    assert admin.probe_human({'ok': False, 'reason': 'no storage-state.json'}) \
        .startswith('no saved session yet')
    assert admin.probe_human({'ok': False, 'http_status': 403}) \
        == 'session rejected — re-login needed'
    assert admin.probe_human({'ok': False, 'reason': 'JSONDecodeError'}) \
        == 'session expired — re-login needed'
    assert admin.probe_human({'ok': False, 'reason': 'URLError'}) == 'SharePoint unreachable'


def test_hours_print_without_trailing_zero(admin):
    assert admin.fmt_h(20.0) == '20'
    assert admin.fmt_h(2.5) == '2.5'


def test_the_health_line_covers_all_five_states(client, admin, auth, monkeypatch):
    def health():
        return client.get('/status.json', headers=auth).get_json()['health']

    assert health() == {'text': 'stale · never ran', 'kind': 'bad'}

    admin.RUNS.write_text(json.dumps({**FRESH_RUN, 'started': iso_ago(hours=2)}) + '\n')
    assert health() == {'text': 'specs fresh · session dead — next run will fail',
                        'kind': 'warn'}

    monkeypatch.setattr(admin, 'probe_session', lambda: {'ok': True, 'user': 'Jane'})
    monkeypatch.setattr(admin, '_probe_cache', (0.0, None))
    assert health() == {'text': 'specs fresh · 2 h ago', 'kind': 'ok'}

    admin.PAUSED.write_text(json.dumps({'reason': 'manual pause'}))
    assert health() == {'text': 'paused', 'kind': 'bad'}          # beats freshness

    monkeypatch.setattr(admin, 'proc', Running())
    assert health() == {'text': 'task running', 'kind': 'run'}    # beats everything


def test_a_run_that_could_not_push_is_not_a_fresh_vault(client, admin, auth, monkeypatch):
    """The whole point of the pill: a commit stuck in the container is not a
    synced vault, and green over it is how five changed pages went unnoticed."""
    monkeypatch.setattr(admin, 'probe_session', lambda: {'ok': True, 'user': 'Jane'})
    admin.RUNS.write_text(json.dumps(
        {**FRESH_RUN, 'outcome': 'push_failed', 'changed': 5,
         'started': iso_ago(hours=2)}) + '\n')
    s = client.get('/status.json', headers=auth).get_json()
    assert s['health'] == {'text': 'stale · committed but not pushed — GitHub is behind',
                           'kind': 'bad'}
    assert s['last_success'] is None
    assert s['last_run']['counts'] == '1 added · 5 changed · 0 removed'


def test_each_run_carries_its_own_wording(client, admin, auth):
    admin.RUNS.write_text(json.dumps(
        {'started': iso_ago(hours=2), 'outcome': 'export_rc_2'}) + '\n')
    s = client.get('/status.json', headers=auth).get_json()
    row, = s['runs']
    # the browser paints outcome_text; it never maps a raw code to a phrase
    assert row['outcome_text'] == 'session dead — re-login needed'
    assert row['outcome_kind'] == 'bad'
    assert row['ago'] == '2 h ago'
    assert row['counts'] == 'nothing changed upstream'


def test_the_last_sync_publishes_its_counts_as_a_sentence(client, admin, auth):
    admin.RUNS.write_text(
        json.dumps({**FRESH_RUN, 'started': iso_ago(hours=2), 'changed': 3}) + '\n'
        + json.dumps({'started': iso_ago(hours=1), 'outcome': 'no_changes'}) + '\n')
    s = client.get('/status.json', headers=auth).get_json()
    assert s['last_success']['counts'] == 'nothing changed upstream'
    assert s['runs'][1]['counts'] == '1 added · 3 changed · 0 removed'


def test_the_pause_publishes_its_reason_and_its_age(client, admin, auth):
    admin.PAUSED.write_text(json.dumps({'reason': 'manual pause', 'at': iso_ago(hours=2)}))
    s = client.get('/status.json', headers=auth).get_json()
    assert s['paused']['reason'] == 'manual pause'
    assert s['paused_since'] == '2 h ago'
    # and the schedule card must not promise a next run that will not happen
    assert s['next_run'] == 'paused'


def test_a_hand_written_pause_file_still_names_a_reason(client, admin, auth):
    admin.PAUSED.write_text('stop it, debugging')
    s = client.get('/status.json', headers=auth).get_json()
    assert s['paused']['reason'] == 'stop it, debugging'
    assert s['paused_since'] is None


def test_the_session_card_facts_are_worded_server_side(client, admin, auth):
    s = client.get('/status.json', headers=auth).get_json()
    assert s['session'] == {'ok': False, 'user': None,
                            'phrase': 'no saved session yet — re-login needed',
                            'state_age': 'no saved session'}
    assert s['login_attempts_24h'] == 0


def test_the_last_relogin_result_is_words_not_a_code(client, admin, auth):
    (admin.DATA / 'app-log.jsonl').write_text(
        json.dumps({'ts': iso_ago(hours=3), 'event': 'login_started'}) + '\n'
        + json.dumps({'ts': iso_ago(hours=3), 'event': 'login_finished',
                      'outcome': 'rc_2'}) + '\n')
    last = client.get('/status.json', headers=auth).get_json()['last_login']
    assert last == {'text': 'failed — see log', 'ago': '3 h ago'}


def test_a_blocked_relogin_names_the_lockout_not_the_pause(client, admin, auth):
    (admin.DATA / 'app-log.jsonl').write_text(
        json.dumps({'ts': iso_ago(hours=1), 'event': 'login_finished',
                    'outcome': 'rc_7'}) + '\n')
    last = client.get('/status.json', headers=auth).get_json()['last_login']
    assert last['text'] == 'blocked — the account is locked out'


# ----------------------------------------------------------- buttons & cards

def test_a_pause_leaves_run_now_alone_but_a_lockout_does_not(client, admin, auth):
    admin.PAUSED.write_text(json.dumps({'reason': 'manual pause', 'at': iso_ago(hours=1)}))
    s = client.get('/status.json', headers=auth).get_json()
    assert s['paused'] and s['lockout'] is False    # the pause holds the schedule only
    admin.PAUSED.write_text(json.dumps({'reason': 'AADSTS50053 smart lockout',
                                        'at': iso_ago(hours=1)}))
    assert client.get('/status.json', headers=auth).get_json()['lockout'] is True


def test_the_schedule_card_facts_are_published(client, admin, auth, monkeypatch):
    monkeypatch.setattr(admin, 'RUN_AT', '23:59')
    s = client.get('/status.json', headers=auth).get_json()
    assert s['run_at'] == '23:59' and s['next_run'] == 'later today'
    admin.SCHEDULE.write_text(json.dumps(
        {'lastRunDate': datetime.now().date().isoformat()}))
    assert client.get('/status.json', headers=auth).get_json()['next_run'] \
        == 'done for today, next tomorrow'


def test_the_footer_links_do_not_hard_code_the_vault(client, admin, auth, monkeypatch):
    monkeypatch.setattr(admin, 'VAULT_WEB', 'https://github.com/you/notes')
    assert client.get('/status.json', headers=auth).get_json()['vault_web'] \
        == 'https://github.com/you/notes'


def test_the_vault_link_is_derived_from_the_repo_url(admin):
    for repo in ('git@github.com:you/notes.git', 'https://github.com/you/notes.git',
                 'https://github.com/you/notes'):
        assert admin.vault_web_from_repo(repo) == 'https://github.com/you/notes', repo
    assert admin.vault_web_from_repo('') == ''


def test_run_now_while_paused_overrides_the_pause(client, admin, auth, monkeypatch):
    admin.PAUSED.write_text(json.dumps({'reason': 'manual pause', 'at': iso_ago(hours=1)}))
    spawned = []
    monkeypatch.setattr(admin, 'spawn', lambda *a: spawned.append(a) or True)
    assert client.post('/run', headers=auth).status_code == 200
    assert spawned == [(['sync.py', '--ignore-pause'], 'sync')]
    assert admin.PAUSED.exists()                    # the run does not end the pause


def test_run_now_is_refused_while_locked_out(client, admin, auth, monkeypatch):
    admin.PAUSED.write_text(json.dumps({'reason': 'AADSTS50053 smart lockout',
                                        'at': iso_ago(hours=1)}))
    spawned = []
    monkeypatch.setattr(admin, 'spawn', lambda *a: spawned.append(a) or True)
    assert client.post('/run', headers=auth).status_code == 200
    assert spawned == []


def test_every_action_answers_the_new_state_not_a_redirect(client, admin, auth, monkeypatch):
    monkeypatch.setattr(admin, 'spawn', lambda *a: True)
    shape = set(client.get('/status.json', headers=auth).get_json())
    for path in ('/run', '/login', '/code', '/pause', '/resume', '/schedule'):
        r = client.post(path, data={'code': '123456'}, headers=auth)
        assert r.status_code == 200, path
        # one shape for the poll and for every button, so the browser needs one
        # parser and one state setter
        assert set(r.get_json()) == shape, path


def test_the_pause_button_round_trip_reports_its_own_result(client, admin, auth):
    assert client.post('/pause', headers=auth).get_json()['paused']['reason'] \
        == 'manual pause'
    assert client.post('/resume', headers=auth).get_json()['paused'] is None


def test_signing_out_still_navigates(client, auth):
    # a plain form POST, so the browser follows the redirect and Flask decides
    # what an anonymous visitor sees next
    r = client.post('/logout', headers=auth)
    assert r.status_code == 302 and r.headers['Location'] == '/'


# ------------------------------------------------------------------- scheduler

class StopLoop(BaseException):
    """Breaks the scheduler's endless loop. Not an Exception, or the loop's own
    error handler would swallow it and spin forever."""


def _stop_after_one_pass(*_):
    raise StopLoop


def _one_scheduler_pass(admin, monkeypatch):
    monkeypatch.setattr(admin, 'RUN_AT', '00:00')          # always past due
    monkeypatch.setattr(admin.time, 'sleep', _stop_after_one_pass)
    spawned = []
    monkeypatch.setattr(admin, 'spawn', lambda *a: spawned.append(a) or True)
    try:
        admin.scheduler()
    except StopLoop:
        pass
    return spawned


def test_the_morning_run_stays_suppressed_while_paused(admin, monkeypatch):
    admin.PAUSED.write_text(json.dumps({'reason': 'manual pause', 'at': iso_ago(hours=1)}))
    assert _one_scheduler_pass(admin, monkeypatch) == []
    assert not admin.SCHEDULE.exists()             # and it is not marked as done today


def test_the_morning_run_does_not_borrow_the_manual_override(admin, monkeypatch):
    assert _one_scheduler_pass(admin, monkeypatch) == [(['sync.py'], 'sync')]


# ------------------------------------------------------- the configured clock

WARSAW = {'time': '08:30', 'tz': 'Europe/Warsaw'}


def _standing_at(admin, monkeypatch, utc_moment):
    """Freeze the app at one real instant, whatever clock the host runs on."""
    moment = datetime.fromisoformat(utc_moment)
    monkeypatch.setattr(admin, 'now_in', lambda tz: moment.astimezone(tz))
    monkeypatch.setattr(admin.time, 'time', lambda: moment.timestamp())


def test_the_run_is_due_at_one_wall_clock_time_on_both_sides_of_a_dst_change(
        admin, monkeypatch):
    admin.SCHEDULE_SETTINGS.write_text(json.dumps(WARSAW))
    # 08:30 in Warsaw is 07:30 UTC while the offset is +1, and 06:30 UTC while it
    # is +2 — an offset stored instead of a zone name would fire an hour out for
    # half the year
    for moment, due in [('2026-01-15T07:29:00+00:00', False),
                        ('2026-01-15T07:30:00+00:00', True),
                        ('2026-07-15T06:29:00+00:00', False),
                        ('2026-07-15T06:30:00+00:00', True)]:
        with monkeypatch.context() as m:
            _standing_at(admin, m, moment)
            assert admin.schedule_state()['due'] is due, moment


def test_the_once_per_day_guard_turns_over_at_the_configured_zones_midnight(
        admin, monkeypatch):
    # 00:30 on the 16th in Warsaw is still the 15th in UTC: read the wrong clock
    # here and the run either doubles up or goes missing on that day
    admin.SCHEDULE_SETTINGS.write_text(json.dumps({'time': '00:15', 'tz': 'Europe/Warsaw'}))
    _standing_at(admin, monkeypatch, '2026-01-15T23:30:00+00:00')
    assert admin.schedule_state()['today'] == '2026-01-16'
    admin.SCHEDULE.write_text(json.dumps({'lastRunDate': '2026-01-15'}))
    assert _one_scheduler_pass(admin, monkeypatch) == [(['sync.py'], 'sync')]
    assert json.loads(admin.SCHEDULE.read_text())['lastRunDate'] == '2026-01-16'
    assert _one_scheduler_pass(admin, monkeypatch) == []      # and not a second time


def test_the_schedule_card_names_the_zone_the_time_is_read_in(client, admin, auth,
                                                              monkeypatch):
    admin.SCHEDULE_SETTINGS.write_text(json.dumps(WARSAW))
    _standing_at(admin, monkeypatch, '2026-01-15T07:00:00+00:00')     # 08:00 in Warsaw
    s = client.get('/status.json', headers=auth).get_json()
    assert (s['run_at'], s['timezone']) == ('08:30', 'Europe/Warsaw')
    assert s['next_run'] == 'later today'


def test_a_run_timestamp_reads_on_the_configured_clock(client, admin, auth, monkeypatch):
    admin.SCHEDULE_SETTINGS.write_text(json.dumps(WARSAW))
    _standing_at(admin, monkeypatch, '2026-07-15T09:00:00+00:00')
    admin.RUNS.write_text(
        json.dumps({**FRESH_RUN, 'started': '2026-07-15T06:05:00+00:00'}) + '\n')
    row = client.get('/status.json', headers=auth).get_json()['runs'][0]
    assert row['when'] == '2026-07-15 08:05 CEST'


def test_saving_a_time_that_has_gone_by_waits_for_tomorrow(client, admin, auth,
                                                           monkeypatch):
    _standing_at(admin, monkeypatch, '2026-01-15T09:00:00+00:00')     # 10:00 in Warsaw
    s = client.post('/schedule', data=WARSAW, headers=auth).get_json()
    assert (s['run_at'], s['timezone']) == ('08:30', 'Europe/Warsaw')
    assert s['next_run'] == 'first run at the new time is tomorrow'
    assert _one_scheduler_pass(admin, monkeypatch) == []
    assert not admin.SCHEDULE.exists()          # and today is not claimed as done
    _standing_at(admin, monkeypatch, '2026-01-16T07:30:00+00:00')     # 08:30 next day
    assert _one_scheduler_pass(admin, monkeypatch) == [(['sync.py'], 'sync')]


def test_saving_a_time_inside_a_skipped_hour_still_does_not_start_a_run(
        client, admin, auth, monkeypatch):
    # 02:30 does not exist in Warsaw on the morning the clocks go forward. Judging
    # "has it gone by" on an absolute epoch on one side and a wall clock on the
    # other made these two disagree, and the save started a real sync a second
    # after the click.
    _standing_at(admin, monkeypatch, '2026-03-29T01:15:00+00:00')     # 03:15 CEST
    s = client.post('/schedule', data={'time': '02:30', 'tz': 'Europe/Warsaw'},
                    headers=auth).get_json()
    assert s['next_run'] == 'first run at the new time is tomorrow'
    assert _one_scheduler_pass(admin, monkeypatch) == []
    assert not admin.SCHEDULE.exists()


def test_the_suppressed_day_does_not_outlive_itself(client, admin, auth, monkeypatch):
    # the suppression is stamped as one date, so a clock that steps backwards (or a
    # hand-edited file) can stall at most that day instead of every day until it
    # catches up
    admin.SCHEDULE_SETTINGS.write_text(json.dumps(
        {'time': '08:30', 'tz': 'Europe/Warsaw', 'skipDate': '2026-02-01'}))
    _standing_at(admin, monkeypatch, '2026-01-15T07:30:00+00:00')     # 08:30 Warsaw
    assert _one_scheduler_pass(admin, monkeypatch) == [(['sync.py'], 'sync')]


def test_the_saved_time_is_normalised_and_outlives_the_process(client, admin, auth):
    s = client.post('/schedule', data={'time': '7:05', 'tz': 'Europe/Warsaw'},
                    headers=auth).get_json()
    assert s['run_at'] == '07:05'
    # nothing is cached in the process: the file on disk is the whole setting
    assert client.get('/status.json', headers=auth).get_json()['run_at'] == '07:05'
    assert json.loads(admin.SCHEDULE_SETTINGS.read_text())['tz'] == 'Europe/Warsaw'


def test_an_unusable_time_or_zone_is_refused_and_changes_nothing(client, admin, auth):
    for bad in [{'time': '25:00'}, {'time': 'lunchtime'}, {'time': ''},
                {'time': '08:30', 'tz': 'Mars/Olympus'},
                {'time': '08:30', 'tz': '../../etc/localtime'}]:
        assert client.post('/schedule', data=bad, headers=auth).get_json()['notice'], bad
    assert not admin.SCHEDULE_SETTINGS.exists()


def test_with_nothing_saved_the_env_default_is_still_in_charge(client, admin, auth,
                                                              monkeypatch):
    monkeypatch.setattr(admin, 'RUN_AT', '06:45')
    s = client.get('/status.json', headers=auth).get_json()
    assert s['run_at'] == '06:45'
    assert s['timezone']                    # named, whatever clock the host is on
    # and a hand-typed env var must not leave the scheduler erroring every minute
    monkeypatch.setattr(admin, 'RUN_AT', 'half past eight')
    assert client.get('/status.json', headers=auth).get_json()['run_at'] == '08:30'
    assert not admin.SCHEDULE_SETTINGS.exists()


# --------------------------------------------------------------------- auth

def test_query_string_token_no_longer_authenticates(client, auth):
    assert client.get('/status.json?token=test-token').status_code == 403
    assert client.get('/status.json', headers=auth).status_code == 200


def test_cookie_is_derived_and_signs_in(client, admin, bundle):
    r = client.post('/auth', data={'token': 'test-token'})
    assert r.status_code == 302
    assert 'test-token' not in r.headers['Set-Cookie']
    issued = r.headers['Set-Cookie'].split('t=', 1)[1].split(';', 1)[0]
    assert admin.session_valid(issued)
    client.set_cookie('t', issued)
    assert '<div id="root">' in client.get('/').data.decode()


def test_expired_session_cookie_is_rejected(client, admin):
    exp = int(time.time()) - 1
    client.set_cookie('t', f'{exp}.{admin.session_mac(exp)}')
    assert 'Sign in' in client.get('/').data.decode()


def test_raw_token_cookie_is_rejected(client):
    client.set_cookie('t', 'test-token')
    assert 'Sign in' in client.get('/').data.decode()


def test_repeated_wrong_tokens_hit_a_per_ip_cap(client, admin):
    for _ in range(admin.FAIL_CAP):
        assert client.post('/auth', data={'token': 'nope'}).status_code == 200
    assert client.post('/auth', data={'token': 'test-token'}).status_code == 429


def test_wrong_header_tokens_count_against_the_cap(client, admin):
    for _ in range(admin.FAIL_CAP - 1):
        assert client.get('/status.json',
                          headers={'X-Admin-Token': 'nope'}).status_code == 403
    # the guess that crosses the cap — and everything after, even the right
    # token — answers 429, so automation sees "throttled", not "wrong token"
    assert client.get('/status.json',
                      headers={'X-Admin-Token': 'nope'}).status_code == 429
    assert client.get('/status.json',
                      headers={'X-Admin-Token': 'test-token'}).status_code == 429


def test_spoofed_forwarded_for_does_not_dodge_the_cap(client, admin):
    # Traefik appends the real IP last; the client-supplied first hop must
    # not hand every guess a fresh throttle bucket
    for i in range(admin.FAIL_CAP):
        client.post('/auth', data={'token': 'nope'},
                    headers={'X-Forwarded-For': f'10.0.0.{i}, 203.0.113.7'})
    r = client.post('/auth', data={'token': 'nope'},
                    headers={'X-Forwarded-For': 'somewhere-else, 203.0.113.7'})
    assert r.status_code == 429
    assert list(admin.FAILURES) == ['203.0.113.7']


def test_stale_failure_buckets_are_dropped(admin):
    admin.FAILURES['198.51.100.9'] = [time.time() - admin.FAIL_WINDOW - 1]
    assert not admin.too_many_failures('198.51.100.9')
    assert '198.51.100.9' not in admin.FAILURES


def test_non_ascii_token_is_a_clean_rejection_not_a_500(client):
    r = client.post('/auth', data={'token': 'sésame-öuvre'})
    assert r.status_code == 200 and 'Wrong token' in r.data.decode()
    assert client.get('/status.json',
                      headers={'X-Admin-Token': 'sésame'}).status_code == 403
    client.set_cookie('t', 'sésame')
    assert 'Sign in' in client.get('/').data.decode()


def test_cross_site_posts_are_rejected_but_curl_still_works(client, admin, auth):
    assert client.post('/pause', headers={**auth,
                       'Origin': 'https://evil.example'}).status_code == 403
    assert client.post('/pause', headers={**auth,
                       'Sec-Fetch-Site': 'cross-site'}).status_code == 403
    assert not admin.PAUSED.exists()
    same = {**auth, 'Origin': 'http://localhost', 'Sec-Fetch-Site': 'same-origin'}
    assert client.post('/pause', headers=same).status_code == 200
    assert admin.PAUSED.exists()
    assert client.post('/resume', headers=auth).status_code == 200   # bare curl path
    assert not admin.PAUSED.exists()


def test_a_dev_server_origin_is_still_cross_site(client, admin, auth):
    # The vite dev server sits on another port, so its Origin does not match this
    # host. The proxy restates Origin as the flask target to get past this; if dev
    # ever breaks here the fix is the proxy config, never a looser check.
    assert client.post('/pause', headers={**auth, 'Origin': 'http://localhost:5173'}
                       ).status_code == 403
    assert not admin.PAUSED.exists()


def test_responses_are_never_cached(client, auth, bundle):
    assert client.get('/', headers=auth).headers['Cache-Control'] == 'no-store'
    assert client.get('/status.json', headers=auth).headers['Cache-Control'] == 'no-store'
    # the hashed assets too: a shared cache holding the admin UI is still the
    # admin UI sitting somewhere nobody handed it to
    assert client.get('/assets/index-abc123.js',
                      headers=auth).headers['Cache-Control'] == 'no-store'


def test_the_referrer_policy_does_not_null_the_origin_header(client, auth, bundle):
    """A browser told 'no-referrer' stamps navigational form POSTs with
    Origin: null, which the cross-site check rejects — so sign-in and sign-out
    both die. Only a real browser reproduces it: the test client sends no Origin
    at all, so the check short-circuits and everything here passes either way.
    This pins the one policy value that breaks it.
    """
    assert client.get('/', headers=auth).headers['Referrer-Policy'] != 'no-referrer'


# --------------------------------------------------------------------- races

def test_status_survives_racy_sentinels(client, admin, auth, monkeypatch):
    monkeypatch.setattr(admin, 'proc', Running())
    admin.PENDING.write_text('{"hint": "torn wri')                 # interrupted write
    admin.RUNS.write_text('{"started": "2026-07-31T08:00:00+00:00", "outco\n')
    assert client.get('/status.json', headers=auth).status_code == 200


def test_pending_vanishing_mid_request_yields_none(client, admin, auth, monkeypatch):
    monkeypatch.setattr(admin, 'proc', Running())                  # runner alive, file gone
    s = client.get('/status.json', headers=auth).get_json()
    assert s['pending'] is None and s['running'] is True


def test_pending_of_a_dead_runner_is_dropped(client, admin, auth):
    admin.PENDING.write_text(json.dumps({'hint': 'code', 'deadline_epoch': 0}))
    s = client.get('/status.json', headers=auth).get_json()
    assert s['pending'] is None
    assert not admin.PENDING.exists()


def test_hand_edited_scalar_sentinels_do_not_break_the_status(client, admin, auth,
                                                             monkeypatch):
    monkeypatch.setattr(admin, 'proc', Running())
    admin.PENDING.write_text('123')
    admin.LAST_RUN.write_text('"oops"')
    admin.ATTEMPTS.write_text(json.dumps([time.time()]))   # a list is still fine
    s = client.get('/status.json', headers=auth).get_json()
    assert s['pending'] is None and s['last_run'] is None
    assert s['login_attempts_24h'] == 1


def test_hand_edited_jsonl_scalar_rows_do_not_break_the_status(client, admin, auth):
    admin.RUNS.write_text('42\n"paused"\n[1, 2]\n')      # a bare {} is covered below
    (admin.DATA / 'app-log.jsonl').write_text('"login_finished"\n[3]\n')
    s = client.get('/status.json', headers=auth).get_json()
    assert s['runs'] == [] and s['last_login'] is None


def test_hand_edited_login_attempts_with_strings_do_not_500(client, admin, auth):
    admin.ATTEMPTS.write_text(json.dumps(['2026-07-31T08:00:00', time.time()]))
    r = client.get('/status.json', headers=auth)
    assert r.status_code == 200
    assert r.get_json()['login_attempts_24h'] == 1


def test_a_hand_edited_schedule_setting_falls_back_instead_of_500ing(client, admin, auth):
    # the only stored file whose contents reach strptime() and a timezone lookup,
    # so it is the one that can take the dashboard down and stop the daily sync
    for bad in ['5', '"08:30"', '[1, 2]', 'not json at all',
                '{"time": 830, "tz": ["Europe/Warsaw"], "skipDate": true}',
                '{"time": "08:30", "tz": "' + 'a/' * 400 + 'z"}',
                '{"tz": {}}', '{"time": null, "tz": 5}']:
        admin.SCHEDULE_SETTINGS.write_text(bad)
        r = client.get('/status.json', headers=auth)
        assert r.status_code == 200, bad
        assert r.get_json()['run_at'] == '08:30', bad


def test_hand_written_pause_timestamp_without_offset_still_reads_as_an_age(client, admin,
                                                                          auth):
    admin.PAUSED.write_text(json.dumps({'reason': 'debugging', 'at': '2026-07-30T08:00:00'}))
    s = client.get('/status.json', headers=auth).get_json()
    assert s['paused_since'].endswith('ago')


# ------------------------------------------------------------- probe caching

def test_the_session_probe_is_not_re_run_on_every_poll(client, admin, auth, monkeypatch):
    calls = []
    monkeypatch.setattr(admin, 'probe_session',
                        lambda: calls.append(1) or {'ok': True, 'user': 'Jane'})
    for _ in range(3):
        assert client.get('/status.json', headers=auth).get_json()['session']['ok']
    # a polling browser must not buy a 10 s sharepoint round trip per tick
    assert len(calls) == 1
    # but the cache has to expire, or the card freezes on one answer forever
    monkeypatch.setattr(admin, '_probe_cache',
                        (time.time() - admin.PROBE_TTL - 1, {'ok': True, 'user': 'Jane'}))
    client.get('/status.json', headers=auth)
    assert len(calls) == 2


def test_a_finished_run_drops_the_cached_session_answer(client, admin, auth, monkeypatch):
    calls = []
    monkeypatch.setattr(admin, 'probe_session',
                        lambda: calls.append(1) or {'ok': len(calls) > 1, 'user': 'Jane'})
    assert client.get('/status.json', headers=auth).get_json()['session']['ok'] is False
    # a sign-in that just finished may have replaced the saved session; leaving the
    # dead answer cached puts "re-login needed" next to a button, and clicking it
    # again spends one of five daily attempts
    admin.forget_session_probe()
    assert client.get('/status.json', headers=auth).get_json()['session']['ok'] is True


def test_a_run_row_with_no_fields_does_not_take_the_admin_down(client, admin, auth):
    admin.RUNS.write_text('{}\n')
    s = client.get('/status.json', headers=auth).get_json()
    assert s['runs'][0]['outcome_text'] == 'never ran'
    assert client.post('/pause', headers=auth).status_code == 200
