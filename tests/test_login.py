"""The Microsoft sign-in: the step it reports, the per-account daily allowance,
and what happens to a code Microsoft rejects.

The runner half runs against a fake page — no Chromium, no network. What is under
test is the state machine over what is on screen, and all that needs is a screen
that changes.
"""
import json
import time

from conftest import Running


# ------------------------------------------------------- the runner's own loop

class Loc:
    def __init__(self, page, sel):
        self.page, self.sel = page, sel

    @property
    def first(self):
        return self

    def is_visible(self):
        return self.page.shows(self.sel)


class FakePage:
    """A sign-in that asks for the email, then keeps asking for a code until the
    right one arrives — which is exactly what Microsoft does with a wrong code."""

    def __init__(self, good_code):
        self.good = good_code
        self.url = 'https://login.microsoftonline.com/common/oauth2/authorize'
        self.screen = 'email'
        self.codes = []
        self.keyboard = self

    def shows(self, sel):
        if 'email' in sel:
            return self.screen == 'email'
        if 'otc' in sel or 'OTC' in sel:
            return self.screen == 'code'
        return False

    def locator(self, sel):
        return Loc(self, sel)

    def content(self):
        return '<html>sign in</html>'

    def fill(self, sel, value):
        if self.screen == 'email':
            self.screen = 'code'
            return
        self.codes.append(value)
        if value == self.good:
            self.url = 'https://example.sharepoint.com/sites/x'

    def press(self, _key):
        pass

    def wait_for_timeout(self, _ms):
        pass

    def click(self, _sel):
        pass


def answers(*sequence):
    """Stands in for a human handing codes over, and records what it was told."""
    it = iter(sequence)
    calls = []

    def fake(hint, deadline, rejected=False, moved_on=None):
        calls.append({'hint': hint, 'rejected': rejected, 'deadline': deadline})
        return next(it, None)

    fake.calls = calls
    return fake


def _no_grace(exporter, monkeypatch, *codes):
    # the grace is Microsoft's round trip; here the fake page has already answered
    monkeypatch.setattr(exporter, 'CODE_GRACE', 0)
    monkeypatch.setattr(exporter, '_await_code', answers(*codes))
    monkeypatch.setattr(exporter, 'account_email', lambda: 'a@example.com')
    return exporter._await_code


def test_a_rejected_code_can_be_typed_again_inside_the_same_attempt(exporter, monkeypatch):
    await_code = _no_grace(exporter, monkeypatch, '111111', '222222')
    page = FakePage(good_code='222222')

    assert exporter.try_login(page) is True
    assert page.codes == ['111111', '222222']
    # the second prompt says the first code was rejected, instead of the runner
    # sitting on the code screen until its deadline and reporting a timeout
    assert await_code.calls[0]['rejected'] is False
    assert await_code.calls[1]['rejected'] is True
    assert 'rejected' in await_code.calls[1]['hint']
    # and all of that was one attempt out of the daily allowance, not two
    assert len(json.loads(exporter.ATTEMPTS.read_text())['a@example.com']) == 1


def test_the_retries_share_the_sign_ins_own_deadline(exporter, monkeypatch):
    await_code = _no_grace(exporter, monkeypatch, '111111', '222222')
    exporter.try_login(FakePage(good_code='222222'))
    # not a second timer: the later prompt cannot outlive the first one's window
    assert await_code.calls[1]['deadline'] == await_code.calls[0]['deadline']


def test_a_run_of_rejected_codes_ends_the_attempt_with_a_reason(exporter, monkeypatch):
    await_code = _no_grace(exporter, monkeypatch, '1', '2', '3', '4')
    page = FakePage(good_code='999999')

    assert exporter.try_login(page) is False
    assert page.codes == ['1', '2', '3']            # it gives up, it does not loop
    assert len(await_code.calls) == 3
    assert 'rejected' in json.loads(exporter.LOGIN_RESULT.read_text())['reason']


def test_a_code_that_never_arrives_ends_the_attempt_with_a_reason(exporter, monkeypatch):
    _no_grace(exporter, monkeypatch)                # nothing is ever submitted
    assert exporter.try_login(FakePage(good_code='222222')) is False
    assert 'in time' in json.loads(exporter.LOGIN_RESULT.read_text())['reason']


def test_a_stale_failure_reason_is_dropped_when_the_next_attempt_starts(exporter,
                                                                        monkeypatch):
    exporter.LOGIN_RESULT.write_text(json.dumps({'reason': 'from the last attempt'}))
    _no_grace(exporter, monkeypatch, '222222')
    assert exporter.try_login(FakePage(good_code='222222')) is True
    assert not exporter.LOGIN_RESULT.exists()


def test_the_prompt_carries_the_rejection_and_the_shared_deadline(exporter, monkeypatch):
    published = {}

    def answer(_seconds):
        published.update(json.loads(exporter.PENDING.read_text()))
        exporter.OTP_ANSWER.write_text(json.dumps({'code': '424242'}))

    monkeypatch.setattr(exporter.time, 'sleep', answer)
    until = time.time() + 60
    assert exporter._await_code('that code was rejected', until, rejected=True) == '424242'
    assert published['rejected'] is True
    assert published['deadline_epoch'] <= until


def test_the_wait_for_a_code_ends_when_the_screen_it_waits_on_is_gone(exporter):
    out = exporter._await_code('type it', time.time() + 600, moved_on=lambda: True)
    assert out is exporter.SCREEN_MOVED_ON
    # and the prompt is withdrawn, not left standing for a code nobody needs
    assert not exporter.PENDING.exists()


def test_a_code_accepted_slowly_is_not_reported_as_a_rejection(exporter, monkeypatch):
    """The code screen can outlive the code that satisfied it.

    Treating that as a rejection threw away a session that actually worked and
    spent one of the day's attempts to do it, so the wait has to end when the
    screen does.
    """
    monkeypatch.setattr(exporter, 'CODE_GRACE', 0.2)
    monkeypatch.setattr(exporter, 'account_email', lambda: 'a@example.com')

    class Lingering(FakePage):
        """Accepts the right code, but the redirect lands a moment later."""

        def __init__(self, good, delay):
            super().__init__(good)
            self.delay, self.accepted_at = delay, None

        def shows(self, sel):
            if self.accepted_at and time.time() - self.accepted_at > self.delay:
                self.url = 'https://example.sharepoint.com/sites/x'
                self.screen = 'done'
            return super().shows(sel)

        def fill(self, sel, value):
            if self.screen == 'email':
                self.screen = 'code'
                return
            self.codes.append(value)
            if value == self.good:
                self.accepted_at = time.time()

        def wait_for_timeout(self, _ms):
            time.sleep(0.02)

    prompts = []

    def await_code(hint, deadline, rejected=False, moved_on=None):
        prompts.append(rejected)
        if len(prompts) == 1:
            return '483920'
        while time.time() < deadline:    # nobody retypes a code they already sent
            if moved_on and moved_on():
                return exporter.SCREEN_MOVED_ON
            time.sleep(0.02)
        return None

    monkeypatch.setattr(exporter, '_await_code', await_code)
    page = Lingering('483920', 0.6)
    assert exporter.try_login(page) is True
    assert page.codes == ['483920']          # the good code went in exactly once
    assert prompts == [False, True]          # a retry was offered, then withdrawn
    assert not exporter.LOGIN_RESULT.exists()


# ------------------------------------------------------ the per-account allowance

def test_each_account_carries_its_own_daily_allowance(exporter):
    for _ in range(exporter.LOGIN_BUDGET):
        assert exporter._login_budget_ok('a@example.com')
    assert not exporter._login_budget_ok('a@example.com')
    # switching account is not refused for what the previous one spent
    assert exporter._login_budget_ok('b@example.com')


def test_one_mailbox_typed_with_a_capital_shares_the_one_allowance(exporter):
    for _ in range(exporter.LOGIN_BUDGET):
        assert exporter._login_budget_ok('a@example.com')
    # Microsoft ignores the capitals, so a second allowance here would be five
    # more real sign-ins against the account the brake exists to keep unlocked
    assert not exporter._login_budget_ok('A@example.com')
    assert not exporter._login_budget_ok('a@Example.Com')


def test_the_account_reader_settles_the_capitals_for_both_halves(admin, exporter,
                                                                monkeypatch):
    monkeypatch.setattr(admin, 'ACCOUNT', exporter.ACCOUNT)
    exporter.ACCOUNT.write_text(json.dumps({'email': ' Jane@Example.Com '}))
    assert admin.account_email() == exporter.account_email() == 'jane@example.com'


def test_a_dressed_up_address_cannot_open_a_second_allowance(client, admin, exporter,
                                                             auth, monkeypatch):
    """The allowance is a dict key, and the mailbox it protects is not.

    Asserted on the bucket rather than on the validator's answer: the thing that
    must stay true is that one mailbox has one allowance, however the address was
    dressed up on the way in.
    """
    spawned = []
    monkeypatch.setattr(admin, 'spawn', lambda *a: spawned.append(a) or True)
    admin.ACCOUNT.write_text(json.dumps({'email': 'a@example.com'}))
    for _ in range(exporter.LOGIN_BUDGET):
        assert exporter._login_budget_ok(admin.account_email())
    # a capital, a zero-width space, a soft hyphen, a bidi isolate, DEL — none of
    # these is a different mailbox to Microsoft, so none may carry its own five
    for variant in ('A@example.com', 'a​@example.com', 'a\xad@example.com',
                    'a⁦@example.com', 'a\x7f@example.com'):
        client.post('/login', data={'email': variant}, headers=auth)
        assert list(json.loads(exporter.ATTEMPTS.read_text())) == ['a@example.com'], variant
    # and none of them got as far as starting a sign-in
    assert spawned == []


def test_attempts_older_than_a_day_stop_counting(exporter):
    exporter.ATTEMPTS.write_text(json.dumps({'a@example.com': [time.time() - 86_401] * 3}))
    assert exporter._login_budget_ok('a@example.com')


def test_an_attempt_log_from_before_the_split_still_brakes(exporter):
    exporter.ATTEMPTS.write_text(json.dumps([time.time()] * exporter.LOGIN_BUDGET))
    assert not exporter._login_budget_ok('a@example.com')


def test_a_hand_edited_attempt_log_does_not_stop_a_sign_in(exporter):
    exporter.ATTEMPTS.write_text('not json at all')
    assert exporter._login_budget_ok('a@example.com')


def test_an_allowance_of_the_wrong_shape_does_not_take_the_admin_down(admin):
    # the whole dashboard renders through this, so a scalar where a list belongs
    # must read as nothing spent rather than 500 every page and every action
    admin.ATTEMPTS.write_text(json.dumps({'a@example.com': 3}))
    assert admin.login_budget('a@example.com')['used'] == 0


# ----------------------------------------------------------------- the account

def test_the_saved_account_wins_over_the_environment(exporter, monkeypatch):
    monkeypatch.setenv('MS_EMAIL', 'env@example.com')
    assert exporter.account_email() == 'env@example.com'
    exporter.ACCOUNT.write_text(json.dumps({'email': 'saved@example.com'}))
    assert exporter.account_email() == 'saved@example.com'
    # and a torn or hand-edited file falls back rather than signing in as nobody
    exporter.ACCOUNT.write_text('{"email": "sav')
    assert exporter.account_email() == 'env@example.com'


def test_the_admin_and_the_runner_read_the_account_the_same_way(admin, exporter,
                                                               monkeypatch):
    monkeypatch.setattr(admin, 'ACCOUNT', exporter.ACCOUNT)
    monkeypatch.setenv('MS_EMAIL', 'env@example.com')
    assert admin.account_email() == exporter.account_email() == 'env@example.com'
    exporter.ACCOUNT.write_text(json.dumps({'email': 'saved@example.com'}))
    assert admin.account_email() == exporter.account_email() == 'saved@example.com'


def test_starting_a_sign_in_saves_the_account_it_is_for(client, admin, auth, monkeypatch):
    spawned = []
    monkeypatch.setattr(admin, 'spawn', lambda *a: spawned.append(a) or True)
    s = client.post('/login', data={'email': 'other@example.com'}, headers=auth).get_json()
    assert s['account_email'] == 'other@example.com'
    assert spawned == [(['export.py', '--login-only'], 'login')]
    # written down, so a container restart still signs in as this account
    assert json.loads(admin.ACCOUNT.read_text()) == {'email': 'other@example.com'}


def test_a_junk_account_email_is_refused_before_anything_starts(client, admin, auth,
                                                               monkeypatch):
    spawned = []
    monkeypatch.setattr(admin, 'spawn', lambda *a: spawned.append(a) or True)
    for bad in ('nope', 'a b@example.com', '@example.com', 'a@example', 'a@example.com\n.x'):
        r = client.post('/login', data={'email': bad}, headers=auth)
        assert 'email address' in r.get_json()['notice']['text'], bad
    assert spawned == [] and not admin.ACCOUNT.exists()


def test_a_sign_in_with_no_account_at_all_says_so_instead_of_starting(client, admin,
                                                                     auth, monkeypatch):
    spawned = []
    monkeypatch.setattr(admin, 'spawn', lambda *a: spawned.append(a) or True)
    s = client.post('/login', headers=auth).get_json()
    assert 'account email' in s['notice']['text']
    assert spawned == [] and s['login']['phase'] == 'idle'


# -------------------------------------------------------------- what the card sees

def phase(client, auth):
    return client.get('/status.json', headers=auth).get_json()['login']


def test_the_sign_in_publishes_the_step_it_is_on(client, admin, auth, monkeypatch):
    monkeypatch.setattr(admin, 'spawn', lambda *a: True)
    admin.ACCOUNT.write_text(json.dumps({'email': 'a@example.com'}))
    assert phase(client, auth)['phase'] == 'idle'

    client.post('/login', headers=auth)
    monkeypatch.setattr(admin, 'proc', Running())
    assert phase(client, auth)['phase'] == 'starting'

    admin.PENDING.write_text(json.dumps({'hint': 'code sent', 'deadline_epoch': 0}))
    assert phase(client, auth)['phase'] == 'awaiting_code'

    client.post('/code', data={'code': '424242'}, headers=auth)
    assert phase(client, auth)['phase'] == 'verifying'

    # the runner takes the code and drops the prompt; the card must not fall back
    # to "starting" for the seconds Microsoft spends deciding
    admin.OTP_ANSWER.unlink()
    admin.PENDING.unlink()
    assert phase(client, auth)['phase'] == 'verifying'

    monkeypatch.setattr(admin, 'proc', None)
    admin.finish_login(0)
    assert phase(client, auth) == {'phase': 'succeeded', 'reason': None, 'rejected': False}


def test_a_failed_sign_in_publishes_the_reason_the_runner_left(client, admin, auth):
    admin.LOGIN_RESULT.write_text(json.dumps({'reason': 'AADSTS50126 — wrong password'}))
    admin.finish_login(2)
    assert phase(client, auth) == {'phase': 'failed', 'rejected': False,
                                   'reason': 'AADSTS50126 — wrong password'}


def test_a_sign_in_killed_without_a_reason_still_names_one(client, admin, auth):
    admin.finish_login(7)
    assert phase(client, auth)['reason'] == 'blocked — the account is locked out'
    admin.finish_login(2)
    assert phase(client, auth)['reason'].startswith('the sign-in did not go through')


def test_a_rejected_code_reaches_the_card(client, admin, auth, monkeypatch):
    monkeypatch.setattr(admin, 'proc', Running())
    admin.PENDING.write_text(json.dumps({'hint': 'that code was rejected — try again',
                                         'rejected': True, 'deadline_epoch': 0}))
    s = client.get('/status.json', headers=auth).get_json()
    assert s['login'] == {'phase': 'awaiting_code', 'reason': None, 'rejected': True}
    # the wording is the runner's, so the browser never maps a state to a phrase
    assert 'rejected' in s['pending']['hint']


def test_the_login_state_answers_on_its_own_too(client, auth):
    assert client.get('/login-state.json', headers=auth).get_json() \
        == {'phase': 'idle', 'reason': None, 'rejected': False}
    assert client.get('/login-state.json').status_code == 403


def test_the_remaining_allowance_is_published_with_the_used_count(client, admin, auth):
    admin.ACCOUNT.write_text(json.dumps({'email': 'a@example.com'}))
    admin.ATTEMPTS.write_text(json.dumps({'a@example.com': [time.time()]}))
    s = client.get('/status.json', headers=auth).get_json()
    assert s['login_budget'] == {'used': 1, 'left': 4,
                                 'text': '1 of 5 daily login attempts used'}
    assert s['login_attempts_24h'] == 1
    admin.ACCOUNT.write_text(json.dumps({'email': 'b@example.com'}))
    assert client.get('/status.json', headers=auth).get_json()['login_budget']['left'] == 5


def test_a_saved_session_survives_until_a_different_address_is_confirmed(
        client, admin, auth, monkeypatch):
    """Nothing throws the saved session away on its own any more.

    There is no endpoint for it: offering to sign in as somebody else must cost
    nothing until the human confirms who, and confirming a different address is
    what replaces the session — one request, in /login.
    """
    spawned = []
    monkeypatch.setattr(admin, 'spawn', lambda *a: spawned.append(a) or True)
    admin.ACCOUNT.write_text(json.dumps({'email': 'a@example.com'}))
    admin.STATE.write_text(json.dumps({'cookies': []}))
    admin.finish_login(0)                                  # a tick from a past sign-in
    assert '/ms-signout' not in {r.rule for r in admin.app.url_map.iter_rules()}
    assert admin.STATE.exists()
    # the same account signing in again keeps the session it already has
    assert client.post('/login', data={'email': 'a@example.com'},
                       headers=auth).status_code == 200
    assert admin.STATE.exists()
    # a different one cannot reuse it, so confirming the switch discards it
    s = client.post('/login', data={'email': 'b@example.com'}, headers=auth).get_json()
    assert not admin.STATE.exists()
    assert s['account_email'] == 'b@example.com'
    assert spawned == [(['export.py', '--login-only'], 'login'),
                       (['export.py', '--login-only'], 'login')]


def test_a_switch_that_is_refused_leaves_the_live_session_alone(client, admin, auth,
                                                                monkeypatch):
    """A no is a no on the way in, not after the damage.

    Discarding the session, rewriting the account and then refusing the request
    left the operator signed out of an account they had never asked to leave, with
    no sign-in started and nothing to sign back in with.
    """
    spawned = []
    monkeypatch.setattr(admin, 'spawn', lambda *a: spawned.append(a) or True)
    admin.ACCOUNT.write_text(json.dumps({'email': 'a@example.com'}))
    admin.STATE.write_text(json.dumps({'cookies': []}))

    def refused_switch(notice_words):
        s = client.post('/login', data={'email': 'b@example.com'}, headers=auth).get_json()
        assert notice_words in s['notice']['text'], notice_words
        assert json.loads(admin.ACCOUNT.read_text()) == {'email': 'a@example.com'}
        assert json.loads(admin.STATE.read_text()) == {'cookies': []}
        assert spawned == []

    # the account being switched to has spent its own allowance today
    admin.ATTEMPTS.write_text(json.dumps({'b@example.com': [time.time()] * admin.LOGIN_BUDGET}))
    refused_switch('No sign-in attempts left today')
    admin.ATTEMPTS.unlink()

    # Microsoft locked the account out: retrying is what deepens a lockout, so the
    # switch is refused here rather than inside a runner we would have paid for
    admin.PAUSED.write_text(json.dumps({'reason': 'AADSTS50053 smart lockout'}))
    refused_switch('locked out')
    admin.PAUSED.unlink()

    # and a task already running: the session must survive being told to wait
    monkeypatch.setattr(admin, 'proc', Running())
    refused_switch('already running')
