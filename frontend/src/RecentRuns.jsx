import { Table } from '@mantine/core';
import { IconHistory } from '@tabler/icons-react';
import {
  ChevronCell, DeltaCell, DeltaHead, OutcomeCell, Row, WhenCell,
} from './RunRow.jsx';
import { Empty, Panel } from './ui.jsx';

const SHOWN = 5;

/** The dashboard's glance at history: did the last few runs go through, and did
 *  they change anything. Deliberately not the runs page — no commit hash, no
 *  actions, no clearing. Those columns only matter once you are actually digging,
 *  and that is what the page they live on is for.
 *
 *  It carries no link to that page either: the Runs tab sits a few pixels above
 *  this panel and is the way there. A second route to the same page only makes
 *  the tab look like it does something else.
 */
export default function RecentRuns({ status, go }) {
  const runs = status.runs.slice(0, SHOWN);

  return (
    <Panel title="Recent activity">
      {runs.length ? (
        <Table.ScrollContainer minWidth={360}>
          {/* Rules between the rows rather than a zebra, for the reason the runs
              page gives: shaded alternate rows read as a state a row is in. */}
          <Table highlightOnHover verticalSpacing={6}>
            <Table.Thead>
              <Table.Tr>
                <Table.Th>when</Table.Th>
                <Table.Th>outcome</Table.Th>
                <DeltaHead />
                <Table.Th />
              </Table.Tr>
            </Table.Thead>
            <Table.Tbody>
              {runs.map((r, i) => (
                <Row key={r.run_id || r.started || i} run={r}>
                  <WhenCell run={r} go={go} />
                  <OutcomeCell run={r} />
                  <DeltaCell run={r} />
                  <ChevronCell run={r} />
                </Row>
              ))}
            </Table.Tbody>
          </Table>
        </Table.ScrollContainer>
      ) : (
        <Empty icon={<IconHistory size={20} />}>
          Nothing has run yet, so there is no history to show. The daily schedule
          writes the first row here — or Run now above starts a sync immediately.
        </Empty>
      )}
    </Panel>
  );
}
