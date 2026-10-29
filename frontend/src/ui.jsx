import {
  ActionIcon, Card, CopyButton, Group, SimpleGrid, Skeleton, Stack, Text, ThemeIcon,
  Tooltip,
} from '@mantine/core';
import { IconCheck, IconCopy } from '@tabler/icons-react';

/** The type scale of every card, and the only place it is written down.
 *
 *  Three roles is all these screens have: the small-caps label a panel is titled
 *  with, the one value it exists to show, and the dimmed lines around that value.
 *  Spelled per panel they drift by half a point at a time until the page stops
 *  reading as one page.
 */
export const Label = (props) => (
  <Text fz={11.5} fw={600} tt="uppercase" lts="0.07em" c="dimmed" {...props} />
);
export const Value = (props) => <Text fz={16.5} fw={650} {...props} />;
export const Meta = (props) => <Text fz="sm" c="dimmed" {...props} />;

/** The tooltip phrases more than one surface shows, and the only place each is
 *  written down — for the same reason the type scale is here. Spelled per file
 *  they drift a word at a time until two screens describe the same thing
 *  differently. Wording only one surface uses stays with that surface.
 */
export const COMMIT = 'open this commit in the vault repo';
export const BUSY = 'a task is already running';
export const LOCKED = 'account locked out — Microsoft clears this, not Resume';

export function Panel({ title, right, children }) {
  return (
    <Card bg="var(--surface)" withBorder radius={13} p="md">
      <Group justify="space-between" gap="xs" wrap="nowrap" mb={8}>
        <Label>{title}</Label>
        {right}
      </Group>
      {children}
    </Card>
  );
}

/** A tooltip for something that cannot carry one itself.
 *
 *  A disabled control receives neither hover nor focus, so a tooltip on it never
 *  opens and a `title` on it is not shown at all — the one moment the reason
 *  matters is the one moment it is unreadable. Plain text has no focus to receive
 *  either. The wrapper is what the pointer and the Tab key reach instead, and a
 *  control inside it stays genuinely disabled.
 */
export function Hint({ label, children }) {
  return (
    <Tooltip label={label}>
      <span tabIndex={0} aria-label={label} style={{ display: 'inline-flex' }}>
        {children}
      </span>
    </Tooltip>
  );
}

/** Copies one string and says so where it was clicked. Commit hashes and run logs
 *  both end up pasted into a terminal, and selecting either by hand is fiddly. */
export function Copy({ value, what }) {
  return (
    <CopyButton value={value} timeout={1500}>
      {({ copied, copy }) => (
        <Tooltip label={copied ? 'copied' : `copy the ${what}`}>
          <ActionIcon variant="subtle" size="sm" color={copied ? 'green' : 'gray'}
                      aria-label={copied ? 'copied' : `Copy the ${what}`} onClick={copy}>
            {copied ? <IconCheck size={14} /> : <IconCopy size={14} />}
          </ActionIcon>
        </Tooltip>
      )}
    </CopyButton>
  );
}

/** What an empty surface says. A bare "no runs yet" leaves the reader unsure
 *  whether it is empty or broken, so each of these names the reason it is empty
 *  and, where there is one, the way out. */
export function Empty({ icon, action, children }) {
  return (
    <Stack align="center" gap={8} py="xl">
      <ThemeIcon variant="light" color="gray" size={38} radius="xl">{icon}</ThemeIcon>
      <Meta ta="center" maw={430}>{children}</Meta>
      {action}
    </Stack>
  );
}

/** First paint, shaped like what replaces it: the three cards and the history
 *  block below them. A centred spinner reflows the whole page once data lands. */
export const ShellSkeleton = () => (
  <Stack gap="md">
    <SimpleGrid cols={{ base: 1, sm: 3 }} spacing="md">
      <Skeleton h={168} radius={13} />
      <Skeleton h={168} radius={13} />
      <Skeleton h={168} radius={13} />
    </SimpleGrid>
    <Skeleton h={230} radius={13} />
  </Stack>
);

/** Same idea for a run: the back link, the progress card, the log. */
export const RunSkeleton = () => (
  <Stack gap="md">
    <Skeleton h={14} w={90} radius="sm" />
    <Skeleton h={250} radius={13} />
    <Skeleton h={170} radius={13} />
  </Stack>
);
