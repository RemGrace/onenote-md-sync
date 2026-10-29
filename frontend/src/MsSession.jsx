import { useEffect, useState } from 'react';
import { Button, Group, Stack, Text, TextInput, Tooltip } from '@mantine/core';
import {
  IconAlertTriangle, IconCheck, IconCircleFilled, IconUserCog,
} from '@tabler/icons-react';
import { confirmed } from './confirm.js';
import { BUSY, Hint, LOCKED, Meta, Value } from './ui.jsx';

const SWITCH = 'sign in as a different Microsoft account — the live session is kept '
  + 'until a new address is confirmed';
const RELOGIN = 'sign in again — the saved address is filled in and can be changed';

const secondsLeft = (pending) =>
  Math.max(0, Math.round((pending?.deadline_epoch || 0) - Date.now() / 1000));

/** The runner gives up at a fixed moment, so the time left has to tick locally
 *  rather than arrive with each poll. */
function useCountdown(pending) {
  const [left, setLeft] = useState(() => secondsLeft(pending));
  useEffect(() => {
    setLeft(secondsLeft(pending));
    const id = setInterval(() => setLeft(secondsLeft(pending)), 1000);
    return () => clearInterval(id);
  }, [pending?.deadline_epoch]);
  return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`;
}

/** The Microsoft session card, and the whole sign-in in the slot where the button
 *  was: email -> Get code -> the emailed code -> Confirm -> outcome.
 *
 *  The *code* step comes from the server's login phase, not from what was clicked
 *  here: a reload mid-sign-in, or a scheduled run parking on the code screen, both
 *  have to land on it in a tab that clicked nothing. The email step is the opposite
 *  — local intent wins there, because the phase stays 'succeeded' for the life of
 *  the process and deferring to it would close the form the instant it was asked for.
 *
 *  The phrases the server owns — the session, the allowance, why a sign-in failed —
 *  are printed verbatim; only the static tooltip help written above is local.
 *
 *  Offering to sign in as somebody else costs nothing: the form opens over a live
 *  session and Cancel puts it back untouched. Confirming a different address is
 *  the single request that replaces the session, and the server does that as part
 *  of starting the sign-in it was asked for. Because that one request is the whole
 *  cost, it is the one moment worth asking about — and only when there is a live
 *  session to lose and the address really is a different one.
 */
export default function MsSession({ status, act }) {
  const { session, login, login_budget: budget, pending, running } = status;
  const [open, setOpen] = useState(false);
  const [email, setEmail] = useState(status.account_email || '');
  const [code, setCode] = useState('');
  const [clicked, setClicked] = useState(false);   // pressed, backend not back yet
  const [signedIn, setSignedIn] = useState(false);  // the one-shot tick after success
  // this tab's own sign-in: none, asked for, or seen under way on the server
  const [attempt, setAttempt] = useState(null);
  const left = useCountdown(pending);

  const asking = login.phase === 'awaiting_code' || login.phase === 'verifying';
  const sending = clicked || login.phase === 'starting';
  const verifying = clicked || login.phase === 'verifying';

  // a fresh prompt means the last code is spent: leaving it in the field makes a
  // rejected code look like one still being checked
  // guarded on a real prompt: an out-of-order poll can briefly answer with no
  // pending at all, and wiping the field then costs the human a code they are
  // part-way through typing, out of a window that cost one of five attempts
  useEffect(() => { if (pending?.since) setCode(''); }, [pending?.since]);

  // A sign-in this tab asked for and that then worked closes the form and shows a
  // tick for a moment; the alive card underneath is the real answer.
  //
  // Both halves of that are needed. The phase alone cannot say "just now": it stays
  // 'succeeded' for the rest of the process's life, so reading it would slam this
  // form shut the moment it opens. And a phase this tab did not ask for is not this
  // tab's news either — a scheduled run parks on the same code screen and its
  // sign-in can end at any time, which would paint "signed in" straight under a
  // card saying the session is dead.
  //
  // So the attempt is followed to the server and back: asked for here, then seen
  // under way, and only then does a phase that is no longer under way end it.
  useEffect(() => {
    if (!attempt) return;
    if (login.phase === 'starting' || login.phase === 'awaiting_code'
        || login.phase === 'verifying') {
      setAttempt('running');
      // a sign-in now under way makes the previous one's tick a lie: leaving it up
      // would report "signed in" over a form asking for a different account
      setSignedIn(false);
      return;
    }
    if (attempt !== 'running') return;   // still the phase this attempt started from
    setAttempt(null);
    if (login.phase !== 'succeeded') return;   // the failure line is the whole answer
    setOpen(false);
    setSignedIn(true);
  }, [login.phase, attempt]);

  // the tick owns its own window, keyed on the tick and nothing else: hung off the
  // phase, any unrelated transition inside those five seconds cancelled the timer
  // and left the line on screen for good
  useEffect(() => {
    if (!signedIn) return undefined;
    const id = setTimeout(() => setSignedIn(false), 5000);
    return () => clearTimeout(id);
  }, [signedIn]);

  // the two posts that ask for a sign-in, and the only place the attempt is claimed
  // as this tab's: the address, and the code the sign-in it started asked for
  const send = async (url, body) => {
    setClicked(true);
    setAttempt('asked');
    await act(url, body);
    setClicked(false);
  };

  // the saved address every time the form opens: a half-typed one left behind by a
  // cancelled switch is not what the card is offering to do
  const openForm = () => {
    setEmail(status.account_email || '');
    setOpen(true);
  };

  // Only a real switch asks. A dead session has nothing to lose, and sending the
  // address that is already saved is a sign-in again rather than a change of
  // account — asking there would be asking about nothing. No count in the
  // sentence: the allowance is per account and the one being typed may not be the
  // one the count belongs to.
  //
  // Compared case-insensitively because that is what the server decides on: it
  // lower-cases the address on the way in and the one it has saved, so Microsoft's
  // one mailbox is one string however it was typed. Matching case here would put a
  // retyped or autofilled capital behind a dialog promising a dropped session and a
  // spent attempt that the server then does not carry out — and a warning that
  // turns out to be untrue is what teaches people to click through the ones that
  // are not.
  const same = (a, b) => a.trim().toLowerCase() === (b || '').trim().toLowerCase();
  const mayReplaceSession = async (addr) =>
    !(session.ok && !same(addr, status.account_email))
    || confirmed({
      title: 'Sign in as a different account?',
      text: `The live session as ${session.user} is dropped the moment this starts, `
        + "and starting it spends one of the day's sign-in attempts.",
      action: 'Switch account',
    });

  const emailStep = (
    <form onSubmit={async (e) => {
      e.preventDefault();
      const addr = email.trim();
      if (addr && await mayReplaceSession(addr)) send('/login', { email: addr });
    }}>
      <Group gap="xs">
        <TextInput value={email} onChange={(e) => setEmail(e.currentTarget.value)}
                   placeholder="account email" type="email" autoComplete="username"
                   aria-label="account email"
                   size="xs" w={230} disabled={sending} autoFocus />
        <Button type="submit" size="xs" loading={sending} disabled={!email.trim()}>
          Get code
        </Button>
        {sending
          ? <Text fz="xs" c="dimmed">contacting Microsoft…</Text>
          : <Button size="xs" variant="subtle" onClick={() => setOpen(false)}>Cancel</Button>}
      </Group>
      <Text fz="xs" c="dimmed" mt={6}>
        {budget.left
          ? `uses 1 of your ${budget.left} remaining attempts today`
          : `${status.account_email || 'this account'} has no attempts left today — `
            + 'another account has its own'}
      </Text>
    </form>
  );

  const codeStep = (
    <form onSubmit={(e) => {
      e.preventDefault();
      if (code.trim()) send('/code', { code: code.trim() });
    }}>
      <Text fz="sm" c={login.rejected ? 'red' : undefined}>
        {pending?.hint}{pending && <> · <b>{left}</b> left</>}
      </Text>
      <Group gap="xs" mt={6}>
        <TextInput value={code} onChange={(e) => setCode(e.currentTarget.value)}
                   placeholder="code" autoComplete="one-time-code" autoFocus
                   aria-label="emailed code"
                   size="xs" w={120} disabled={verifying} />
        <Button type="submit" size="xs" loading={verifying} disabled={!code.trim()}>
          Confirm
        </Button>
        {verifying && <Text fz="xs" c="dimmed">authorising…</Text>}
      </Group>
    </form>
  );

  // One affordance per state, and it is the same control: a live session offers to
  // become a different account, a dead one offers to sign in again.
  const [action, hint] = session.ok ? ['Switch account', SWITCH] : ['Re-login', RELOGIN];
  // an exhausted allowance must not disable this: opening the field spends nothing,
  // the allowance is per account, and typing a different address is the way out.
  // The server refuses an account that is actually out, and says so in the card.
  // A lockout is the opposite case and disables it whatever the session is doing:
  // the pause it writes is not per account, so the sign-in is refused for every
  // address, and a form that cannot lead anywhere is worse than a stated reason.
  const blocked = running ? BUSY : status.lockout ? LOCKED : null;
  const restStep = blocked ? (
    <Hint label={blocked}>
      <Button variant="default" disabled
              leftSection={<IconUserCog size={16} />}>{action}</Button>
    </Hint>
  ) : (
    <Tooltip label={hint}>
      <Button variant="default" onClick={openForm}
              leftSection={<IconUserCog size={16} />}>{action}</Button>
    </Tooltip>
  );

  return (
    <Stack gap={0}>
      <Value>
        <IconCircleFilled size={9} style={{ verticalAlign: 'middle',
          color: `var(--mantine-color-${session.ok ? 'green' : 'red'}-text)` }} />
        {session.ok ? <> alive as <b>{session.user}</b></> : ` ${session.phrase}`}
      </Value>
      <Meta mt={6}>{status.account_email || 'no account set'} · {session.state_age}</Meta>
      <Meta>{budget.text}</Meta>
      {status.last_login && (
        <Meta>last sign-in: {status.last_login.text} · {status.last_login.ago}</Meta>
      )}
      {login.phase === 'failed' && (
        <Group gap={4} mt={6} wrap="nowrap">
          <IconAlertTriangle size={14} style={{ color: 'var(--mantine-color-red-text)' }} />
          <Text fz="sm" c="red">{login.reason}</Text>
        </Group>
      )}
      {signedIn && (
        <Group gap={4} mt={6} wrap="nowrap">
          <IconCheck size={14} style={{ color: 'var(--mantine-color-green-text)' }} />
          <Text fz="sm" c="green">signed in</Text>
        </Group>
      )}
      <div style={{ marginTop: 10 }}>
        {asking ? codeStep : open || sending ? emailStep : restStep}
      </div>
    </Stack>
  );
}
