import {
  Alert, Anchor, Badge, Button, Group, Loader, SimpleGrid, Stack, Tooltip,
} from '@mantine/core';
import {
  IconInfoCircle, IconPlayerPause, IconPlayerPlay, IconRefresh,
} from '@tabler/icons-react';
import MsSession from './MsSession.jsx';
import RecentRuns from './RecentRuns.jsx';
import { KIND, navigate } from './App.jsx';
import { BUSY, COMMIT, Copy, Hint, LOCKED, Meta, Panel, Value } from './ui.jsx';

// What a pause does and does not stop. It is the whole reason the state belongs
// in this block rather than in a red bar across the page: paused is a fact about
// the schedule, not about the app.
const PAUSE_SCOPE = 'Only the daily scheduled run is suspended. Run now and the '
  + 'Microsoft sign-in still work.';

const RUN_NOW = 'sync now, without waiting for the daily schedule';

export default function Dashboard({ status, act, go }) {
  const { last_success: last, running, run_progress: progress } = status;
  const open = (p) => navigate(go, p);

  return (
    <Stack gap="md">
      {/* one line, and it links through. The phase timeline, the bar and the
          counters have exactly one implementation, on the run's own page. */}
      {progress && (
        <Alert color="blue" py="xs">
          <Group gap="xs">
            <Loader size={12} type="oval" />
            <Anchor fz="sm" href={`/runs/${progress.run_id}`}
                    onClick={open(`runs/${progress.run_id}`)}>{progress.text}</Anchor>
          </Group>
        </Alert>
      )}

      <SimpleGrid cols={{ base: 1, sm: 3 }} spacing="md">
        <Panel title="Last sync">
          {last ? (
            <>
              <Group gap={2}>
                <Badge variant="light" radius="xl" mr={4}
                       color={KIND[last.outcome_kind]}>{last.outcome_text}</Badge>
                {last.commit && (
                  <>
                    <Tooltip label={COMMIT}>
                      <Anchor href={`${status.vault_web}/commit/${last.commit}`}
                              target="_blank" rel="noopener noreferrer"
                              fz="sm">{last.commit}</Anchor>
                    </Tooltip>
                    <Copy value={last.commit} what="vault commit" />
                  </>
                )}
              </Group>
              <Meta mt={6}>{last.ago} · {last.counts}</Meta>
            </>
          ) : <Value>never ran</Value>}
          <Group mt={10}>
            {running ? (
              <Hint label={BUSY}>
                <Button disabled leftSection={<IconRefresh size={16} />}>Run now</Button>
              </Hint>
            ) : status.lockout ? (
              <Hint label={LOCKED}>
                <Button disabled leftSection={<IconRefresh size={16} />}>Run now</Button>
              </Hint>
            ) : (
              <Tooltip label={RUN_NOW}>
                <Button leftSection={<IconRefresh size={16} />}
                        onClick={() => act('/run')}>Run now</Button>
              </Tooltip>
            )}
          </Group>
        </Panel>

        <Panel title="Microsoft session">
          <MsSession status={status} act={act} />
        </Panel>

        <Panel title="Schedule" right={status.paused && (
          <Badge color="red" variant="light" radius="xl"
                 leftSection={<IconPlayerPause size={11} />}>paused</Badge>
        )}>
          <Value>daily at {status.run_at} {status.timezone}</Value>
          <Meta mt={6}>
            {status.paused
              ? `scheduled runs suspended: ${status.paused.reason || 'paused'}`
                + (status.paused_since ? ` — since ${status.paused_since}` : '')
              : `${status.next_run} · one attempt per day, retries are manual`}
          </Meta>
          <Group mt={10} gap="xs">
            {status.paused ? (
              <Tooltip label="let the daily scheduled run happen again">
                <Button leftSection={<IconPlayerPlay size={16} />}
                        onClick={() => act('/resume')}>Resume</Button>
              </Tooltip>
            ) : (
              <Tooltip label="stop the daily scheduled run until it is resumed">
                <Button variant="default" leftSection={<IconPlayerPause size={16} />}
                        onClick={() => act('/pause')}>Pause</Button>
              </Tooltip>
            )}
            {status.paused && (
              <Hint label={PAUSE_SCOPE}>
                <IconInfoCircle size={17} style={{ color: 'var(--mantine-color-dimmed)' }} />
              </Hint>
            )}
          </Group>
        </Panel>
      </SimpleGrid>

      <RecentRuns status={status} go={go} />

      {/* The raw log dump and the status document are gone from here: every run in
          the block above opens its own page, which carries the same log with the
          phase, the progress and the counters around it. */}
      <Meta>
        <Anchor href={status.vault_web} target="_blank" rel="noopener noreferrer"
                fz="sm">vault repo</Anchor>
      </Meta>
    </Stack>
  );
}
