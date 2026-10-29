# any2md is built from the patched clone in vendor/ (local-HTML input support);
# the main any2md checkout stays untouched until its own WIP lands.
# Build in the SAME image family the binary runs in: rust:1-slim is Debian
# trixie (glibc 2.41) and its binary dies on jammy's glibc 2.35 at runtime.
FROM mcr.microsoft.com/playwright/python:v1.60.0-jammy AS any2md
RUN DEBIAN_FRONTEND=noninteractive apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        build-essential curl pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH=/root/.cargo/bin:$PATH
WORKDIR /src
COPY vendor/any2md/ .
RUN cargo build --release

# The admin UI is a Vite bundle. It is built in its own stage so node never
# reaches the runtime image — only the hashed assets cross over. package.json
# first, so a source-only edit reuses the cached npm ci layer.
FROM node:22-slim AS frontend
WORKDIR /src
COPY frontend/package.json frontend/package-lock.json ./
# --ignore-scripts: nothing in the tree needs an install hook, and a build stage
# that runs as root should not execute one on a dependency's behalf
RUN npm ci --ignore-scripts
COPY frontend/ ./
RUN npm run build

# Tag must match requirements.txt playwright==1.60.0, or the bundled browsers
# and the pypi driver disagree.
FROM mcr.microsoft.com/playwright/python:v1.60.0-jammy
# tzdata: the base image ships no /usr/share/zoneinfo, and the schedule stores its
# zone as a name — without the database every named zone falls back to the
# container's own clock and the configured time means nothing.
# DEBIAN_FRONTEND is not optional here: tzdata's postinst asks for a geographic
# area, and with a tty-less builder debconf falls back to the readline frontend
# and blocks on stdin forever. The build does not fail, it hangs — and a hung
# build holds the deploy queue, so every later push silently stops shipping.
RUN DEBIAN_FRONTEND=noninteractive apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        git openssh-client tzdata \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY --from=any2md /src/target/release/any2md /usr/local/bin/any2md
# every top-level module, not just the entry points — a helper left out of this
# list only shows up as an ImportError on the deployed container
COPY export.py convert.py sync.py app.py recon.py pause.py entrypoint.sh ./
# app.py serves this as its static folder; without it GET / answers 503
COPY --from=frontend /src/dist ./frontend/dist
RUN chmod +x entrypoint.sh
ENV ANY2MD_BIN=/usr/local/bin/any2md VAULT_DIR=/vault
EXPOSE 8377
CMD ["./entrypoint.sh"]
