import React from 'react';
import { createRoot } from 'react-dom/client';
import { MantineProvider, Tooltip, createTheme } from '@mantine/core';
import '@mantine/core/styles.css';
import App from './App.jsx';

const theme = createTheme({
  fontFamily: 'system-ui, -apple-system, "Segoe UI", sans-serif',
  defaultRadius: 'md',
  primaryColor: 'blue',
  headings: { sizes: { h1: { fontSize: '19px', fontWeight: 650 } } },
  components: {
    // Every tooltip in the app answers the keyboard and a touch, not only a
    // pointer — set once here, because a reason that needs a mouse to read is
    // the same dead end as the `title` attributes these replaced.
    Tooltip: Tooltip.extend({
      defaultProps: { withArrow: true, events: { hover: true, focus: true, touch: true } },
    }),
  },
});

// The palette the server-rendered dashboard used, kept verbatim so the port is
// not also a redesign. The sign-in form in app.py still carries its own copy of
// these hexes and nothing links the two — they are kept in step by hand. Mantine
// has no "surface above the background" variable of its own, hence --surface:
// cards sit on top of the body colour in both schemes.
const cssVariablesResolver = () => ({
  variables: {},
  light: {
    '--mantine-color-body': '#f4f6f9',
    '--mantine-color-text': '#1b2430',
    '--mantine-color-dimmed': '#68737f',
    '--mantine-color-default-border': '#e2e7ee',
    '--surface': '#ffffff',
  },
  dark: {
    '--mantine-color-body': '#0f141b',
    '--mantine-color-text': '#e7ecf2',
    '--mantine-color-dimmed': '#8b97a5',
    '--mantine-color-default-border': '#27313d',
    '--surface': '#161d27',
  },
});

createRoot(document.getElementById('root')).render(
  <React.StrictMode>
    <MantineProvider theme={theme} defaultColorScheme="auto"
                     cssVariablesResolver={cssVariablesResolver}>
      <App />
    </MantineProvider>
  </React.StrictMode>,
);
