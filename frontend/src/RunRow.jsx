import { Anchor, Badge, Group, Table, Tooltip } from '@mantine/core';
import { IconChevronRight } from '@tabler/icons-react';
import { KIND, navigate } from './App.jsx';
import { COMMIT, Copy, Hint, Meta } from './ui.jsx';
import './rows.css';

const NO_LOG = 'no log was kept for this run — it finished before runs had a page '
  + 'of their own';
const DELTA = 'files added, changed and removed in the vault by this run';

/** The cells the two history surfaces share. The dashboard block and the runs page
 *  choose which of them to show and word their own headers; what a row *is* — one
 *  link into a run, or an inert leftover with no page — is decided once, here.
 */
export const Row = ({ run, children }) => (
  <Table.Tr className={run.run_id ? 'run-row run-row-open' : 'run-row'}>
    {children}
  </Table.Tr>
);

/** The first cell, and the row's link: rows.css stretches this anchor over the
 *  whole row, so every cell beside it is part of the same click target. */
export const WhenCell = ({ run, go }) => (
  <Table.Td>
    {run.run_id ? (
      // the accessible name carries the exact moment the relative one hides, and
      // opens with the visible text so that speaking what is on screen still
      // reaches this link — and it is not a `title`, which would pop the browser's
      // own tooltip over the whole row and fight the real ones on the cells beside it
      <Anchor className="run-row-link" fz="sm"
              aria-label={`${run.ago} — open the run of ${run.when}`}
              href={`/runs/${run.run_id}`} onClick={navigate(go, `runs/${run.run_id}`)}>
        {run.ago}
      </Anchor>
    ) : (
      <Hint label={NO_LOG}><Meta>{run.ago}</Meta></Hint>
    )}
  </Table.Td>
);

export const OutcomeCell = ({ run }) => (
  <Table.Td>
    <Badge variant="light" radius="xl" color={KIND[run.outcome_kind]}>
      {run.outcome_text}
    </Badge>
  </Table.Td>
);

export const CommitCell = ({ run, vaultWeb }) => (
  <Table.Td className="run-row-control">
    {run.commit ? (
      <Group gap={2} wrap="nowrap">
        <Tooltip label={COMMIT}>
          <Anchor fz="sm" href={`${vaultWeb}/commit/${run.commit}`} target="_blank"
                  rel="noopener noreferrer">{run.commit}</Anchor>
        </Tooltip>
        <Copy value={run.commit} what="vault commit" />
      </Group>
    ) : <Meta>—</Meta>}
  </Table.Td>
);

/** What the counters mean, on the header rather than on each cell: it explains a
 *  whole column, and the header sits outside the row's link — a cell lifted above
 *  that link to stay hoverable is a hole in the row's click target. */
export const DeltaHead = () => (
  <Table.Th>
    <Hint label={DELTA}>added / changed / removed</Hint>
  </Table.Th>
);

export const DeltaCell = ({ run }) => (
  <Table.Td>
    <Meta>
      {run.commit
        ? `${run.added ?? 0} / ${run.changed ?? 0} / ${run.removed ?? 0}`
        : '—'}
    </Meta>
  </Table.Td>
);

/** Says the row opens something. It is not the link itself — the whole row is. */
export const ChevronCell = ({ run }) => (
  <Table.Td w={30}>
    {run.run_id && (
      <IconChevronRight size={15} aria-hidden
                        style={{ display: 'block', color: 'var(--mantine-color-dimmed)' }} />
    )}
  </Table.Td>
);
