"""A rejected push is a normal event in this vault: a run takes minutes and a
laptop or an agent's session notes can land on main in the middle of one.

Real git in tmp dirs, on purpose. What can break in the retry is the *order* of
git commands and the state they leave behind — a mocked subprocess would happily
pass while the container sits mid-rebase and every later run refuses to start.
"""
import subprocess

import pytest

import sync as syncer


def git(*args, cwd):
    return subprocess.run(['git', *args], cwd=cwd, capture_output=True, text=True,
                          check=True)


def commit(repo, path, text, msg):
    (repo / path).write_text(text)
    git('add', '-A', cwd=repo)
    git('commit', '-q', '-m', msg, cwd=repo)


@pytest.fixture
def vaults(tmp_path):
    """A bare remote plus two clones: ours, and the other writer's."""
    remote = tmp_path / 'remote.git'
    git('init', '-q', '--bare', '-b', 'main', str(remote), cwd=tmp_path)
    mine, theirs = tmp_path / 'mine', tmp_path / 'theirs'
    for repo in (mine, theirs):
        git('clone', '-q', str(remote), str(repo), cwd=tmp_path)
        git('config', 'user.email', 'sync@test', cwd=repo)
        git('config', 'user.name', 'sync', cwd=repo)
        git('config', 'commit.gpgsign', 'false', cwd=repo)
    commit(mine, 'spec.md', 'one\n', 'first')
    git('push', '-q', '-u', 'origin', 'main', cwd=mine)
    git('pull', '-q', cwd=theirs)
    return mine, theirs, remote


def subjects(repo):
    return git('log', '--format=%s', 'origin/main', cwd=repo).stdout.split()


def test_a_push_rejected_by_another_writer_rebases_and_lands(vaults):
    mine, theirs, _ = vaults
    commit(theirs, 'notes.md', 'session\n', 'theirs')
    git('push', '-q', cwd=theirs)

    commit(mine, 'spec.md', 'two\n', 'ours')       # no fetch: our pull was minutes ago
    assert syncer.push(mine) == ''

    git('fetch', '-q', cwd=mine)
    assert subjects(mine) == ['ours', 'theirs', 'first']


def test_a_conflicting_push_gives_up_without_wedging_the_next_run(vaults):
    """Someone edited the same file in Obsidian. We cannot rebase past that, and
    force-pushing over them is not on the table — but the tree must be left
    clean, or main()'s staged-changes guard stops every later run from retrying.
    """
    mine, theirs, _ = vaults
    commit(theirs, 'spec.md', 'theirs\n', 'theirs')
    git('push', '-q', cwd=theirs)

    commit(mine, 'spec.md', 'ours\n', 'ours')
    assert syncer.push(mine) != ''

    assert not (mine / '.git' / 'rebase-merge').exists()
    assert not (mine / '.git' / 'rebase-apply').exists()
    assert git('status', '--porcelain', cwd=mine).stdout == ''
    assert git('log', '-1', '--format=%s', cwd=mine).stdout.strip() == 'ours'


def test_no_retry_loop_when_the_first_push_works(vaults):
    mine, _, _ = vaults
    commit(mine, 'spec.md', 'two\n', 'ours')
    assert syncer.push(mine, attempts=1) == ''
    git('fetch', '-q', cwd=mine)
    assert subjects(mine) == ['ours', 'first']
