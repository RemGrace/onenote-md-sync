import { useState } from 'react';
import { Button, Group, Select, TextInput } from '@mantine/core';
import { Meta, Panel } from './ui.jsx';

// The browser already knows both answers: which zone this human reads a clock in,
// and every zone name Intl can resolve. Nothing to ship, nothing to keep current.
const BROWSER_ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone;
const ZONES = Intl.supportedValuesOf ? Intl.supportedValuesOf('timeZone') : [BROWSER_ZONE];

/** The one editor for the daily sync time. The dashboard card shows the same two
 *  facts read-only; this is the page where they change.
 *
 *  The zone starts as whatever is saved, and as the browser's own when nothing is
 *  — a fresh deployment has only an unlabelled container clock to offer, which is
 *  the thing this setting exists to replace.
 */
export default function Settings({ status, act }) {
  // the server says outright whether a zone is configured; `timezone` is only the
  // label to print, and on an unconfigured container it reads as an abbreviation
  // that can collide with a real zone name ('UTC' is in the Intl list)
  const saved = status.timezone_saved;
  const [at, setAt] = useState(status.run_at);
  const [zone, setZone] = useState(saved || BROWSER_ZONE);
  const [saving, setSaving] = useState(false);
  // the server resolved the saved name, so it is valid even when this browser's
  // canonical list has only the modern spelling of it — keep it selectable
  const zones = ZONES.includes(zone) ? ZONES : [zone, ...ZONES];

  const save = async (e) => {
    e.preventDefault();
    setSaving(true);
    await act('/schedule', { time: at, tz: zone });
    setSaving(false);
  };

  return (
    <form onSubmit={save}>
      <Panel title="Schedule">
        <Group gap="xs" align="flex-end">
          <TextInput type="time" label="daily sync at" value={at} w={130}
                     onChange={(e) => setAt(e.currentTarget.value)} />
          <Select label="timezone" data={zones} value={zone} searchable w={260}
                  onChange={(v) => v && setZone(v)} />
          <Button type="submit" loading={saving}
                  disabled={at === status.run_at && zone === saved}>
            Save
          </Button>
        </Group>
        <Meta mt={10}>
          {status.next_run} · saving never starts a run: a time that has already
          gone by today first comes round tomorrow.
        </Meta>
      </Panel>
    </form>
  );
}
