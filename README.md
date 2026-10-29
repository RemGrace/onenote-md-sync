# onenote-md-sync

Exports a OneNote notebook stored on SharePoint into Markdown and commits it to a git repo of
notes (for example an Obsidian vault). Playwright drives the way in, `any2md` does the conversion,
a coverage gate decides whether the result is good enough to commit. A small admin UI runs it on
a schedule and handles the Microsoft sign-in.

## Pipeline

```
OneNote notebook ──export.py──▶ data/staging (HTML + JSON + images)
                 ──convert.py──▶ data/out (Markdown) ── coverage gate
                 ──sync.py────▶ <vault>/<VAULT_NOTES_DIR>/ ── one git commit, push
```

| step | in | out |
|---|---|---|
| `export.py` | the notebook, via the saved browser session | `data/staging/<Section>/<Page>.html` + `.json` + inline images |
| `convert.py` | staging | `data/out/**.md` via `any2md`, gated on a character-level comparison with the source |
| `sync.py` | `data/out` | changed notes copied into the vault, one commit |

## Safety rules

These are load-bearing, not decoration:

- the run refuses to start if the vault has staged changes and aborts on a rebase conflict — it
  never force-pushes over hand edits made in the vault;
- nothing reaches the vault unless the coverage gate passed (default 99 %);
- deletions apply only after a run that exported every page it saw — a page that failed to render
  is not a deleted page;
- image names are the content hash, so re-running produces zero git churn;
- a page's workflow status (`(To review)`, `(In progress)`) is stripped from the filename and kept
  in staging metadata, otherwise every status change would rename a note;
- only `<VAULT_NOTES_DIR>` inside the vault is ever touched.

`data/` is a credential — live session, HAR with cookies, raw dumps. Gitignored, `chmod 600`,
never copied into the vault.

## Connect your notebook and vault

Copy `.env.example` to `.env` and fill in:

| variable | what | example |
|---|---|---|
| `SHAREPOINT_SITE` | site that holds the notebook | `https://contoso.sharepoint.com/sites/Team` |
| `NOTEBOOK_PATH` | server-relative folder with the notebook's `.one` files | `/sites/Team/Shared Documents/Specs` |
| `NOTEBOOK_URL` | the notebook as OneNote for the web opens it | copy from the browser address bar |
| `VAULT_REPO` | git URL of the notes repo | `git@github.com:you/notes.git` |
| `VAULT_DIR` | local clone of the vault (`/vault` in docker) | `~/vaults/notes` |
| `VAULT_NOTES_DIR` | subfolder of the vault the sync owns | `specs` |
| `VAULT_WEB` | optional link for the UI; derived from `VAULT_REPO` | `https://github.com/you/notes` |
| `MS_EMAIL` | account to sign in with | `you@example.com` |
| `RUN_AT`, `TZ` | daily run time and its zone | `08:30`, `UTC` |

`export.py` and `recon.py` stop with a clear message when a required variable is missing. The
vault folder `<VAULT_DIR>/<VAULT_NOTES_DIR>` must exist before the first sync; create it and
commit it once by hand.

The browser runs with `locale='fr-FR'` (`export.py`), which matches the notebook it was built for;
the date and status patterns accept both French and English. Change it if your notebook's UI
labels differ.

## Setup (local)

```bash
python3 -m venv .venv && ./.venv/bin/pip install -r requirements.txt
./.venv/bin/playwright install chromium
./.venv/bin/pip install pytest                 # dev only
```

Keep a stable egress IP — Entra treats a changing one as a risk signal. First login is
interactive (`./.venv/bin/python recon.py --fresh`) or through the admin UI.

```bash
./.venv/bin/python sync.py                 # export -> convert -> vault -> commit
./.venv/bin/python sync.py --dry-run       # everything except the commit
./.venv/bin/python sync.py --section Intro --limit 3
```

### Change detection

A section is a `.one` file in the notebook folder (groups are subfolders). `export.py` lists that
folder over SharePoint REST with the session cookies, skips any section whose `len`+`mtime` match
`data/section-state.json` and whose staged copy exists, crawls the rest, then prunes staged pages
and sections that vanished. A flaky section is dropped from the state so the next run re-crawls it.
If the REST listing fails, the run degrades to a full crawl with no skip and no prune.
Force a full crawl: `rm data/section-state.json`.

## The `any2md` patch

`any2md` needs one patch to read a local `.html` file (its `--url` mode blocks `file://` and
private IPs). The patched clone lives in `vendor/any2md` and is built by the Docker image;
`ANY2MD_BIN` points to another binary if you build it yourself (`cargo build --release` in
`vendor/any2md`).

## Admin (`app.py`)

```bash
ADMIN_TOKEN=... ./.venv/bin/python app.py     # http://127.0.0.1:8377/
curl -H "X-Admin-Token: $ADMIN_TOKEN" http://127.0.0.1:8377/status.json
```

Sign in with the token in the form; the browser gets an HttpOnly cookie holding an expiring HMAC
derived from the token, never the token itself. Wrong tokens are compared constant-time and capped
per IP (10 / 15 min, then 429). Served by waitress, so a slow SharePoint probe cannot freeze the
next request.

`app.py` renders only the sign-in form and answers JSON: `/status.json` (whole state, and the
response of every action POST), `/run.json?id=<run_id>` and `GET /runs/<run_id>` for one run,
`/log` for a log tail. Logs live in `data/runs/<run_id>.log`, last 50 kept.

Login: account email → Get code → emailed code → Confirm, inside the session card. Attempts are
capped at 5 per 24 h per account, failures classify by AADSTS code, and `AADSTS50053` (smart
lockout) writes the PAUSED sentinel — runs stop until a human clears it. Account in
`data/ms-account.json`, `MS_EMAIL` as fallback.

Scheduler: one attempt per day past the configured time (Settings tab, `RUN_AT` as fallback);
retries are a human clicking Run now. The PAUSED sentinel blocks scheduled and CLI runs too.

### Front end (`frontend/`)

Vite + React + Mantine. Flask serves the built bundle as its static folder, behind the same
sign-in gate.

```bash
npm --prefix frontend install
npm --prefix frontend run build      # -> frontend/dist, which app.py serves (503 without it)
npm --prefix frontend run dev        # dev, alongside app.py -> http://127.0.0.1:5173
```

Open the dev server on `127.0.0.1`, not `localhost` (cookies are scoped by host). The dev proxy
restates both `Host` and `Origin`, because `cross_site()` rejects a POST whose `Origin`
disagrees with its `Host`.

## Docker

```bash
cp .env.example .env
docker network create dokploy-network     # once; the compose file expects it
docker compose up -d --build
```

The image builds the patched `any2md` and the admin bundle in their own stages. Named volumes:
`data` (the session — a credential) and `vault`. On first boot `entrypoint.sh` materialises the
vault-push deploy key from `DEPLOY_KEY_B64` and clones `VAULT_REPO` — never a token in the remote
URL. Add the public half of that key as a write deploy key on the vault repo. The port is bound
to `127.0.0.1`; put a reverse proxy with HTTPS in front for remote access.

## Runbook

- **Green pill** «specs fresh · N h ago» — nothing to do.
- **Red pill** «stale · last: <outcome>» — open that run's page for its log. Usual causes:
  `pull_failed` (vault diverged — fix by hand, never force-push), `convert_failed` (coverage gate,
  a page changed shape), `export_rc_2` (session died → **Re-login**), `export_rc_8` (a required
  variable is missing), `push_failed` (the commit is in the volume, not on the remote — resolve
  the conflict in the vault, then **Run now** carries it out).
- **Amber banner asking for a code** — type the emailed code within the countdown.
- **Paused** — someone hit Pause or a lockout tripped; **Resume** re-arms the scheduler.
- **Session dead** — **Re-login** (with `M365_PASSWORD` it walks to the code step by itself). Do
  not copy a laptop session into the server volume: a cookie replayed from a new IP is exactly
  what Entra flags.

## Tests

```bash
./.venv/bin/python -m pytest              # python side, no network
(cd vendor/any2md && cargo test)          # the patched converter
```
