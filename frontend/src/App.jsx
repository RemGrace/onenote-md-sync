import { useEffect, useRef, useState } from 'react';
import {
  Alert, AppShell, Badge, Button, Container, Group, Loader, Tabs, Title, Tooltip,
} from '@mantine/core';
import { IconGauge, IconHistory, IconSettings } from '@tabler/icons-react';
import { confirmed } from './confirm.js';
import Dashboard from './Dashboard.jsx';
import RunsPage from './RunsPage.jsx';
import Settings from './Settings.jsx';
import RunPage from './RunPage.jsx';
import { ShellSkeleton } from './ui.jsx';

// What the health pill at the top of the page actually reports.
const HEALTH = 'how fresh the specs in the vault are, plus anything that stops the '
  + 'next run: a task already running, a paused schedule, a dead Microsoft session.';

// outcome_kind / health.kind as the server words it -> a Mantine colour.
export const KIND = { ok: 'green', bad: 'red', warn: 'yellow', run: 'blue', mut: 'gray' };

/** onClick for an internal link that also carries a real href. Only the plain
 *  left-click is taken over; a Cmd/Ctrl/Shift/middle click stays the browser's,
 *  or the href is decoration and "open in a new tab" silently does nothing. */
export const navigate = (go, to) => (e) => {
  if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
  e.preventDefault();
  go(to);
};

/** How often the shell asks for state.
 *
 *  Only two things on these screens move on their own: a run in flight and a
 *  sign-in waiting for its emailed code. Everything else changes because someone
 *  clicked, and a click already answers with the new state — so the fast poll is
 *  for those two and the resting page checks in slowly. A 4 s metronome held
 *  forever was 900 requests an hour per open tab to say "nothing has changed",
 *  and an admin left open on a second monitor is the normal case here.
 */
const BUSY_MS = 4000;
const IDLE_MS = 30_000;

const PAGES = ['dashboard', 'runs', 'settings'];
const path = () => location.pathname.replace(/^\/+|\/+$/g, '') || 'dashboard';

/** Three tab pages, plus one page per run under runs/<id>: pushState and a
 *  popstate listener are the whole router. Every page keeps a real URL, so a
 *  reload, Back, and a bookmarked run all work. */
function usePage() {
  const [route, setRoute] = useState(path);
  useEffect(() => {
    const onPop = () => setRoute(path());
    addEventListener('popstate', onPop);
    return () => removeEventListener('popstate', onPop);
  }, []);
  const go = (p) => {
    // compared against the raw route, not the tab: the Runs tab has to navigate
    // while a run's page is open, and Mantine fires onChange on the active tab too
    if (p === route) return;
    history.pushState(null, '', p === 'dashboard' ? '/' : `/${p}`);
    setRoute(p);
  };
  // not decoded: a run id is a timestamp the server minted, and
  // decodeURIComponent throws on a stray % — during render, with no boundary
  const runId = route.startsWith('runs/') ? route.slice(5) : null;
  // a run's page lives under the Runs tab, so that tab stays the highlighted one
  return [runId ? 'runs' : PAGES.includes(route) ? route : 'dashboard', go, runId];
}

/** Every action answers the same shape /status.json does, so there is one parser
 *  and one state setter for the poll and for the buttons alike. */
async function postAction(url, body) {
  const r = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams(body || {}),
  });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
}

export default function App() {
  const [status, setStatus] = useState(null);
  // The one-shot notice an action came back with. Deliberately not read out of
  // `status` on a poll: it belongs to the click that caused it, and a later poll
  // must not resurrect it.
  const [notice, setNotice] = useState(null);
  const [expired, setExpired] = useState(false);
  const [page, go, runId] = usePage();
  // Bumped by every action. A poll that started before a click must not paint its
  // pre-click snapshot over the state the click just returned — a probe miss makes
  // a tick take seconds, which is long enough for a Pause to appear and vanish.
  const seq = useRef(0);
  // A hidden tab is not being read by anyone, so it asks for nothing at all —
  // and asks the moment it comes back, because what it last painted may be hours
  // old. The browser's own answer to "is this page being looked at", no library
  // and no heartbeat of our own.
  const [visible, setVisible] = useState(() => !document.hidden);
  // The two states worth watching closely; anything else is waiting for a click.
  // Read off `status`, which an action's answer sets too, so pressing Run now
  // speeds the poll up on the spot instead of after the current idle wait.
  const busy = !!status && (status.running || !!status.pending);

  useEffect(() => {
    const onVisibility = () => setVisible(!document.hidden);
    document.addEventListener('visibilitychange', onVisibility);
    return () => document.removeEventListener('visibilitychange', onVisibility);
  }, []);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      const startedAt = seq.current;
      try {
        const r = await fetch('/status.json');
        if (r.status === 403) {
          // The session aged out. In production a reload lands on the Flask
          // sign-in form; under the Vite dev server the page comes from Vite and
          // cannot, so reload at most once and then say it plainly.
          if (sessionStorage.getItem('reloadedOnExpiry')) setExpired(true);
          else {
            sessionStorage.setItem('reloadedOnExpiry', '1');
            location.reload();
          }
          return;
        }
        if (!r.ok) return;
        const s = await r.json();
        sessionStorage.removeItem('reloadedOnExpiry');
        setExpired(false);             // a tick got through, so the session is back
        // never touches the code input's state, and never an action's fresher answer
        if (alive && seq.current === startedAt) setStatus(s);
      } catch {
        /* a dropped tick is not an error worth showing; the next one retries */
      }
    };
    if (!visible) return () => { alive = false; };
    // On entry as well as on the interval: this effect is re-entered when the tab
    // comes back and when the page starts or stops being busy, and each of those
    // is a moment the state on screen is worth refreshing at once.
    tick();
    const id = setInterval(tick, busy ? BUSY_MS : IDLE_MS);
    return () => { alive = false; clearInterval(id); };
  }, [visible, busy]);

  // Returns whether the action landed, because the click is the only feedback
  // the human gets. Redirecting used to make a failure impossible to miss — the
  // browser navigated to the error; answering JSON means saying so here instead,
  // or a full disk turns every button into a no-op that looks like success.
  const act = async (url, body) => {
    seq.current += 1;
    try {
      const s = await postAction(url, body);
      setStatus(s);
      setNotice(s.notice);
      return true;
    } catch (e) {
      setNotice({ kind: 'red', text: `That did not go through (${e.message}) — nothing `
                                     + 'changed. Check the last run log and try again.' });
      return false;
    }
  };

  return (
    <AppShell header={{ height: 96 }} padding="md">
      <AppShell.Header withBorder={false}>
        <Container size={900} px={0}>
          <Group gap="sm" py="sm" wrap="wrap">
            <Title order={1}>onenote-md-sync</Title>
            {status && (
              <Tooltip label={HEALTH} multiline w={290}>
                <Badge size="lg" radius="xl" variant="light" tabIndex={0}
                       color={KIND[status.health.kind]} leftSection={
                         status.running
                           ? <Loader size={10} color={KIND.run} type="oval" /> : null}>
                  {status.health.text}
                </Badge>
              </Tooltip>
            )}
            {/* The one action that is a real form POST rather than a fetch: it
                ends the session, so the answer has to be a page the server
                renders, not JSON this bundle would have nobody left to show.
                That makes its confirmation the odd one too — the submit is
                stopped, and the form is sent by hand once the dialog agrees.
                The element is read before the await: React clears currentTarget
                the moment the handler returns, and it returns at the await.

                Stopping the submit is what makes a broken dialog dangerous here,
                so the failure is caught: the browser's own submit is already
                cancelled, React drops a rejected async handler silently, and
                /logout answers nothing but a POST — a throw with no catch would
                leave a signed-in admin no way out of a month-long cookie except
                devtools. Saying so is enough; the button is still there to press
                again. */}
            <form method="post" action="/logout" style={{ marginLeft: 'auto' }}
                  onSubmit={async (e) => {
                    e.preventDefault();
                    const form = e.currentTarget;
                    try {
                      if (await confirmed({
                        title: 'Sign out?',
                        text: 'The admin token is needed to get back in. The schedule '
                          + 'keeps running either way — signing out stops nothing.',
                        action: 'Sign out',
                      })) form.submit();
                    } catch {
                      setNotice({ kind: 'red', text: 'Could not ask about signing out, '
                                                     + 'so nothing was done. Press it again.' });
                    }
                  }}>
              <Button type="submit" variant="default" size="xs">Sign out</Button>
            </form>
          </Group>
          <Tabs value={page} onChange={go} variant="default">
            <Tabs.List>
              <Tabs.Tab value="dashboard" leftSection={<IconGauge size={16} />}>
                Dashboard
              </Tabs.Tab>
              <Tabs.Tab value="runs" leftSection={<IconHistory size={16} />}>
                Runs
              </Tabs.Tab>
              <Tabs.Tab value="settings" leftSection={<IconSettings size={16} />}>
                Settings
              </Tabs.Tab>
            </Tabs.List>
          </Tabs>
        </Container>
      </AppShell.Header>

      <AppShell.Main>
        <Container size={900} px={0}>
          {notice && (
            <Alert color={notice.kind === 'red' ? 'red' : 'yellow'} mb="md" withCloseButton
                   onClose={() => setNotice(null)}>{notice.text}</Alert>
          )}
          {expired && (
            <Alert color="red" mb="md" title="Session expired">
              Sign in again to keep going.
            </Alert>
          )}
          {!status ? <ShellSkeleton /> : runId
            ? <RunPage id={runId} status={status} go={go} />
            : page === 'dashboard'
              ? <Dashboard status={status} act={act} go={go} />
              : page === 'runs' ? <RunsPage status={status} act={act} go={go} />
              : <Settings status={status} act={act} />}
        </Container>
      </AppShell.Main>
    </AppShell>
  );
}
