import {
  ActionIcon, Anchor, Button, Group, Stack, Table, Tooltip,
} from '@mantine/core';
import { IconHistory, IconTrash } from '@tabler/icons-react';
import { navigate } from './App.jsx';
import { confirmed } from './confirm.js';
import {
  ChevronCell, CommitCell, DeltaCell, DeltaHead, OutcomeCell, Row, WhenCell,
} from './RunRow.jsx';
import { Copy, Empty, Hint, Meta, Panel, Value } from './ui.jsx';

// No count in the sentence: a confirmation that names a wrong number is worse
// than one that names none.
const CLEAR_CONFIRM = 'Delete every run in the history and its log? The health '
  + "indicator will read 'never ran' until the next sync.";

/** One statistic of the history and what it counts. Every one of them is "of the
 *  runs shown": the server tails the last 20 rows and the file may hold more, so
 *  any total here would be a number this page cannot know. */
const Stat = ({ value, children }) => (
  <Stack gap={0}>
    <Value>{value}</Value>
    <Meta>{children}</Meta>
  </Stack>
);

/** The run history in full: what the last runs did, the commit each produced, and
 *  the way into any of them. The dashboard's block answers "is this healthy"; this
 *  page answers "what happened", so the extra columns and every action live here.
 */
export default function RunsPage({ status, act, go }) {
  const runs = status.runs;
  // sorted by the kind the server already publishes for each row, never by a raw
  // outcome code and never as "whatever is left": paused and never-ran are neither
  // healthy nor failures, and counting the failures as the remainder printed both
  // of them as failed — as it would any outcome the server grows later
  const healthy = runs.filter((r) => r.outcome_kind === 'ok');
  const committed = healthy.filter((r) => r.commit).length;
  const quiet = healthy.filter((r) => !r.commit).length;
  const failed = runs.filter((r) => r.outcome_kind === 'bad').length;
  const lastCommit = runs.find((r) => r.commit);

  const deleteRun = async (r) => {
    if (await confirmed({
      title: 'Delete this run?',
      text: `The row for ${r.when} and this run's log go together, and neither comes `
        + 'back. What the run committed stays in the vault.',
      action: 'Delete this run',
    })) act('/runs/delete', r.run_id ? { id: r.run_id } : { started: r.started });
  };

  const clearHistory = async () => {
    if (await confirmed({
      title: 'Clear the history?',
      text: CLEAR_CONFIRM,
      action: 'Delete every run',
    })) act('/runs/clear');
  };

  const clearButton = runs.length ? (
    <Tooltip label="delete every run in the history, rows and logs together">
      <Button variant="default" size="xs" leftSection={<IconTrash size={14} />}
              onClick={clearHistory}>Clear history</Button>
    </Tooltip>
  ) : (
    <Hint label="the history is already empty">
      <Button variant="default" size="xs" disabled
              leftSection={<IconTrash size={14} />}>Clear history</Button>
    </Hint>
  );

  return (
    <Stack gap="md">
      <Panel title="History" right={clearButton}>
        <Group gap={40} wrap="wrap">
          <Stat value={runs.length}>runs shown</Stat>
          <Stat value={committed}>committed</Stat>
          <Stat value={quiet}>no changes</Stat>
          <Stat value={failed}>failed</Stat>
          {lastCommit && (
            <Stack gap={0}>
              <Group gap={2} wrap="nowrap">
                <Value component="span">
                  <Anchor inherit target="_blank" rel="noopener noreferrer"
                          href={`${status.vault_web}/commit/${lastCommit.commit}`}>
                    {lastCommit.commit}
                  </Anchor>
                </Value>
                <Copy value={lastCommit.commit} what="last vault commit" />
              </Group>
              <Meta>last vault commit</Meta>
            </Stack>
          )}
        </Group>
      </Panel>

      {runs.length ? (
        <Table.ScrollContainer minWidth={520}>
          {/* Hover highlight only, no stripes: a zebra shades the first row and
              not the second, which on a two-row history reads as "this one is
              selected" or "this one is current" and means neither. The row
              borders separate the rows; the shading only ever answered a
              question nobody asked. */}
          <Table highlightOnHover bg="var(--surface)" style={{
            border: '1px solid var(--mantine-color-default-border)',
            borderRadius: 13, overflow: 'hidden',
          }}>
            <Table.Thead>
              <Table.Tr>
                <Table.Th>when</Table.Th>
                <Table.Th>outcome</Table.Th>
                <Table.Th>vault commit</Table.Th>
                <DeltaHead />
                <Table.Th />
                <Table.Th />
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {runs.map((r, i) => (
                <Row key={r.run_id || r.started || i} run={r}>
                  <WhenCell run={r} go={go} />
                  <OutcomeCell run={r} />
                  <CommitCell run={r} vaultWeb={status.vault_web} />
                  <DeltaCell run={r} />
                  <ChevronCell run={r} />
                  {/* The class is what makes this clickable at all: rows.css
                      stretches the row's link over every cell, and only a
                      positioned cell sits above it.

                      Always visible, never hover-only — a hover affordance does
                      not exist on touch. One click away is the point: the two
                      other things this row's menu used to offer are both already
                      here, the row itself opens the run and the commit cell
                      copies the commit, so a menu was a step in front of the one
                      thing it alone could do. That step is now the dialog, which
                      is the step worth having. */}
                  <Table.Td className="run-row-control" w={40}>
                    <Tooltip label="delete this run from the history, row and log">
                      <ActionIcon variant="subtle" color="red" size="sm"
                                  aria-label={`Delete the run of ${r.when}`}
                                  onClick={() => deleteRun(r)}>
                        <IconTrash size={16} />
                      </ActionIcon>
                    </Tooltip>
                  </Table.Td>
                </Row>
              ))}
            </Table.Tbody>
          </Table>
        </Table.ScrollContainer>
      ) : (
        <Empty icon={<IconHistory size={20} />} action={
          <Anchor fz="sm" href="/" onClick={navigate(go, 'dashboard')}>
            open the dashboard
          </Anchor>
        }>
          No runs in the history. Every sync writes a row here with its outcome and
          its log, and clearing the history empties the page — Run now on the
          dashboard starts one.
        </Empty>
      )}
    </Stack>
  );
}
