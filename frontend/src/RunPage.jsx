import { Fragment, useEffect, useRef, useState } from 'react';
import {
  Accordion, Alert, Anchor, Badge, Card, Checkbox, Divider, Group, List, Loader,
  Progress, ScrollArea, Stack, Text, ThemeIcon, Tooltip,
} from '@mantine/core';
import {
  IconCheck, IconCircle, IconFileOff, IconLoader2, IconX,
} from '@tabler/icons-react';
import { KIND, navigate } from './App.jsx';
import { COMMIT, Copy, Empty, Label, Meta, RunSkeleton, Value } from './ui.jsx';

// The phase states the server publishes, as a bullet and a colour on the chain.
const BULLET = {
  done: <IconCheck size={13} />,
  active: <IconLoader2 size={13} />,
  pending: <IconCircle size={13} />,
  stopped: <IconX size={13} />,
};
const BULLET_COLOUR = { done: 'green', active: 'blue', pending: 'gray', stopped: 'red' };
const LABEL_COLOUR = { active: 'blue', pending: 'dimmed', stopped: 'red' };

/** The four phases, left to right, joined by a rule — export → convert → vault
 *  delta → commit is a sequence, and read across one line it is four words and
 *  four ticks. Stacked down the card it was four rows of near-empty space in
 *  front of the counters, which are what the page is actually opened for.
 *
 *  Wraps rather than scrolls on a narrow screen: a chain cut off at the edge
 *  hides the phase that failed. */
const Chain = ({ phases }) => (
  <Group gap="xs" wrap="wrap">
    {phases.map((ph, i) => (
      <Fragment key={ph.key}>
        {i > 0 && <Divider w={16} />}
        <Group gap={6} wrap="nowrap">
          <ThemeIcon size={20} radius="xl" color={BULLET_COLOUR[ph.state]}
                     variant={ph.state === 'pending' ? 'light' : 'filled'}>
            {BULLET[ph.state]}
          </ThemeIcon>
          <Text fz="sm" c={LABEL_COLOUR[ph.state]}>{ph.label}</Text>
        </Group>
      </Fragment>
    ))}
  </Group>
);

/** How the sync decides what changed. It is written out here because two of its
 *  outcomes read as bugs otherwise: a run that finishes in seconds with nothing
 *  to show, and a page deleted upstream that is still sitting in the vault. */
const HOW_IT_DECIDES = [
  'Per section, upstream: if SharePoint reports a section unchanged since the last '
  + 'run, it is not crawled at all and the pages already staged from it stand in. '
  + 'That is why a run can finish fast with nothing to show.',
  'Per note, downstream: the converted markdown is hashed and compared with the hash '
  + 'recorded the last time that note synced. A matching hash whose file is still in '
  + 'the vault is skipped; a note with no record is added; a different hash is changed.',
  'Removals are deliberately conservative: a note is deleted from the vault only after '
  + 'a run that saw and rendered every page. A page that failed to render cannot be '
  + 'told apart from one deleted upstream, and guessing wrong destroys vault content — '
  + 'so a partial run suppresses every deletion and says so above.',
];

/** One run: the phase it is in, how far through, and its whole log.
 *
 *  Polls only while the run is live and stops as soon as the server says it has
 *  settled, so a finished run costs one request.
 */
export default function RunPage({ id, status, go }) {
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const [follow, setFollow] = useState(false);   // off until the human asks for it
  const logBox = useRef(null);

  useEffect(() => {
    let alive = true;
    let timer;
    const tick = async () => {
      try {
        const r = await fetch(`/run.json?id=${encodeURIComponent(id)}`);
        // an error body is not always JSON — the auth gate answers plain text
        const body = await r.json().catch(() => ({}));
        if (!alive) return;
        if (!r.ok) { setError(body.error || `HTTP ${r.status}`); return; }
        setError(null);
        setData(body);
        if (body.live) timer = setTimeout(tick, 2000);
      } catch {
        if (alive) timer = setTimeout(tick, 4000);   // a dropped tick just retries
      }
    };
    tick();
    return () => { alive = false; clearTimeout(timer); };
  }, [id]);

  // Only when the checkbox is on, and only ever downwards: no scroll-position
  // guessing, so reading the middle of a live log is never fought over.
  useEffect(() => {
    if (follow && logBox.current) logBox.current.scrollTop = logBox.current.scrollHeight;
  }, [data, follow]);

  const back = (
    <Anchor href="/runs" fz="sm" onClick={navigate(go, 'runs')}>
      ← Runs
    </Anchor>
  );

  if (error) {
    return (
      <Stack gap="sm">
        {back}
        <Alert color="gray" title="No progress for this run">{error}</Alert>
      </Stack>
    );
  }
  if (!data) return <RunSkeleton />;

  const p = data.progress;
  const run = data.run;

  return (
    <Stack gap="md">
      <Group justify="space-between">
        {back}
        {/* The moment on the reader's own clock, with the age beside it — the raw
            UTC ISO string that used to stand here is neither. `title` keeps the
            exact stamp for anyone matching this page against a log line. */}
        <Meta title={run?.started}>
          {run?.when || id}{run?.ago ? ` · ${run.ago}` : ''}
        </Meta>
      </Group>

      <Card bg="var(--surface)" withBorder radius={13} p="md">
        {/* a run that ended before the export started has no phases and no
            counters, only an outcome and a log — its page is those two */}
        {p && <Chain phases={p.phases} />}

        <Group justify="space-between" wrap="nowrap" mt={p ? 'md' : 0}>
          <Group gap="xs">
            {data.live && <Loader size={12} type="oval" />}
            <Value>{p ? p.headline : 'Ended before the export started'}</Value>
            {run && !data.live && (
              <Badge variant="light" radius="xl"
                     color={KIND[run.outcome_kind]}>{run.outcome_text}</Badge>
            )}
          </Group>
          {p && p.percent !== null && (
            <Meta>{p.done} / {p.total} · {p.percent}%</Meta>
          )}
        </Group>

        {!p ? null : p.percent !== null ? (
          <Progress value={p.percent} mt={8} radius="xl"
                    animated={data.live} color={p.ended ? 'green' : 'blue'} />
        ) : p.indeterminate && (
          <Meta mt={8}>
            no percentage until every section has been crawled and the page count
            is known — the counters below are what is certain so far
          </Meta>
        )}

        {p && (
          <Stack gap={2} mt={10}>
            {p.counters.map((c) => <Meta key={c}>{c}</Meta>)}
            {p.delta && <Text fz="sm" mt={4}>{p.delta}</Text>}
            {p.commit && (
              <Group gap={2} wrap="nowrap">
                <Tooltip label={COMMIT}>
                  <Anchor href={`${status.vault_web}/commit/${p.commit}`} target="_blank"
                          rel="noopener noreferrer" fz="sm">vault commit {p.commit}</Anchor>
                </Tooltip>
                <Copy value={p.commit} what="vault commit" />
              </Group>
            )}
          </Stack>
        )}

        {p?.note && <Alert color="yellow" mt="sm">{p.note}</Alert>}

        {p && (
          <Accordion variant="filled" mt="sm" chevronPosition="left">
            <Accordion.Item value="how">
              <Accordion.Control px={0}>
                <Text fz="sm">How this run decided what changed</Text>
              </Accordion.Control>
              <Accordion.Panel px={0}>
                <List size="sm" spacing="xs">
                  {HOW_IT_DECIDES.map((line) => <List.Item key={line}>{line}</List.Item>)}
                </List>
              </Accordion.Panel>
            </Accordion.Item>
          </Accordion>
        )}
      </Card>

      <Group justify="space-between">
        <Label>full log</Label>
        <Group gap="xs">
          {data.log && <Copy value={data.log} what="whole log" />}
          <Checkbox size="xs" label="follow" checked={follow}
                    onChange={(e) => setFollow(e.currentTarget.checked)} />
        </Group>
      </Group>
      <Card bg="var(--surface)" withBorder radius={13} p="sm">
        {data.log ? (
          <ScrollArea h={420} viewportRef={logBox} type="auto">
            <pre style={{ margin: 0, fontSize: 12.5, whiteSpace: 'pre-wrap',
                          wordBreak: 'break-word' }}>{data.log}</pre>
          </ScrollArea>
        ) : (
          <Empty icon={<IconFileOff size={20} />}>
            {data.live
              ? 'Nothing logged yet — this run has only just started, and the lines '
                + 'appear here as it prints them.'
              : 'No log was kept for this run. Logs are pruned as newer runs arrive, '
                + 'and clearing the history deletes them with their rows.'}
          </Empty>
        )}
      </Card>
    </Stack>
  );
}
