"""The image copies a hand-written file list, so a new module is invisible until
the container starts and dies on the import. Cheaper to notice here.
"""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def test_the_image_copies_every_top_level_module():
    copied = set()
    for line in (ROOT / 'Dockerfile').read_text().splitlines():
        if line.startswith('COPY') and line.rstrip().endswith('./'):
            copied |= set(re.findall(r'\S+\.py', line))
    on_disk = {p.name for p in ROOT.glob('*.py')}
    assert on_disk - copied == set(), 'add these to the Dockerfile COPY line'


def test_the_image_builds_the_frontend_and_ships_the_bundle():
    """The UI is a node build, but the runtime image has no node — so a broken
    recipe here only shows up as a 503 on the deployed container. This gate stays
    node-free itself; it reads the recipe.
    """
    docker = (ROOT / 'Dockerfile').read_text()
    assert 'FROM node:' in docker, 'the bundle needs a node build stage'
    assert 'npm ci' in docker and 'npm run build' in docker
    # and the built output has to land where app.py's static folder looks
    assert 'COPY --from=frontend /src/dist ./frontend/dist' in docker
    # that COPY path is only right while vite keeps its default outDir; setting
    # one would leave every test green and serve a 503
    assert 'outDir' not in (ROOT / 'frontend' / 'vite.config.js').read_text()


def test_the_image_ships_the_timezone_database():
    """The base image has no /usr/share/zoneinfo at all. Without the database a
    zone stored as a name resolves to nothing and the schedule silently falls back
    to the container's own clock — an hour or more out, with every test green.

    Read off the runtime stage alone, with continuations joined and comment lines
    dropped. Each of those earns its keep: the word appears in the comment above
    the install as well, commenting the install out would otherwise leave this
    green, an earlier stage installing it would not put it in the shipped image,
    and this file's own prevailing style puts packages on a continuation line.
    """
    text = (ROOT / 'Dockerfile').read_text()
    runtime = text[text.rindex('\nFROM '):]         # the last stage is the one that runs
    runtime = re.sub(r'\\\n\s*', ' ', runtime)      # one logical line per command
    installs = [ln for ln in runtime.splitlines()
                if 'apt-get install' in ln and not ln.lstrip().startswith('#')]
    assert installs, 'the runtime stage installs nothing at all'
    assert any('tzdata' in ln for ln in installs), installs


def test_every_package_install_is_noninteractive():
    """tzdata's postinst asks which geographic area you live in. A builder has no
    tty, so debconf falls back to the readline frontend and waits on stdin — the
    build does not fail, it hangs, and a hung build holds the deploy queue, so
    every later merge quietly stops reaching production while GitHub stays green.

    Blanket rule rather than a tzdata special case: the next prompting package
    would hang exactly the same way, in whichever stage it lands.
    """
    text = re.sub(r'\\\n\s*', ' ', (ROOT / 'Dockerfile').read_text())
    installs = [ln for ln in text.splitlines()
                if 'apt-get install' in ln and not ln.lstrip().startswith('#')]
    assert installs, 'no package install found at all — has the recipe moved?'
    blocking = [ln.strip() for ln in installs
                if 'DEBIAN_FRONTEND=noninteractive apt-get install' not in ln]
    assert not blocking, ('these can block on a debconf prompt and hang the '
                          f'build: {blocking}')


def test_the_dev_proxy_restates_the_origin_header():
    """The one invariant in the diff with no other check behind it.

    changeOrigin rewrites Host only, so without an explicit Origin every POST from
    the dev server is rejected as cross-site — and the tempting fix is to loosen
    the server instead, which would drop the CSRF guard in production.
    """
    cfg = (ROOT / 'frontend' / 'vite.config.js').read_text()
    assert 'changeOrigin: true' in cfg
    assert 'Origin:' in cfg, 'the proxy must restate Origin as the flask target'
