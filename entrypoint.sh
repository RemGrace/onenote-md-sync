#!/bin/sh
# Container entry: materialise the vault-push deploy key from the environment,
# clone the vault on first boot, then hand off to the admin app.
set -eu

if [ -n "${DEPLOY_KEY_B64:-}" ]; then
    mkdir -p /keys
    echo "$DEPLOY_KEY_B64" | base64 -d > /keys/deploy_key
    chmod 600 /keys/deploy_key
    export GIT_SSH_COMMAND="ssh -i /keys/deploy_key -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new"
fi

VAULT="${VAULT_DIR:-/vault}"
if [ ! -d "$VAULT/.git" ]; then
    : "${VAULT_REPO:?set VAULT_REPO to the vault repo git URL}"
    echo "vault is empty — cloning $VAULT_REPO"
    git clone "$VAULT_REPO" "$VAULT"
fi
git config --global user.name  "${GIT_AUTHOR_NAME:-onenote-md-sync}"
git config --global user.email "${GIT_AUTHOR_EMAIL:-onenote-md-sync@localhost}"
git config --global --add safe.directory "$VAULT"

mkdir -p /app/data && chmod 700 /app/data
exec python app.py
