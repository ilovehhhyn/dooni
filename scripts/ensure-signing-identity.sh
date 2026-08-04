#!/bin/sh
#
# Creates (once) a local self-signed code-signing identity for dooni and prints
# its name on stdout.
#
# Why this exists: dooni used to be ad-hoc signed (`codesign --sign -`). macOS
# records an app's Accessibility grant against its designated requirement, and
# for an ad-hoc signature that requirement is pinned to the binary's code hash.
# Every rebuild produced a new hash, so the grant silently stopped applying —
# the row in System Settings stayed checked while AXIsProcessTrusted() returned
# false. Signing with a stable certificate makes the designated requirement
# reference the certificate instead of the binary, so the grant survives
# rebuilds and only needs to be given once.
#
# The key lives in its own keychain rather than the login keychain so that it
# can be created and used without prompting, and without touching credentials
# the user cares about. It is a local signing key for a locally built app; it
# grants no authority beyond this machine.

set -eu

IDENTITY_NAME="dooni local signing"
KEYCHAIN_NAME="dooni-signing.keychain"
SUPPORT_DIR="$HOME/Library/Application Support/dooni"
PASSWORD_FILE="$SUPPORT_DIR/signing-keychain-password"

log() { printf '%s\n' "$1" >&2; }

keychain_path() {
  # macOS appends -db to keychains it creates on modern releases.
  if [ -f "$HOME/Library/Keychains/$KEYCHAIN_NAME-db" ]; then
    printf '%s\n' "$HOME/Library/Keychains/$KEYCHAIN_NAME-db"
  else
    printf '%s\n' "$HOME/Library/Keychains/$KEYCHAIN_NAME"
  fi
}

# Prints the identity's SHA-1 hash, empty if it is not present yet.
#
# `find-identity -v` is deliberately not used: it lists only identities whose
# certificate chains to a trusted root, and a self-signed certificate reports
# CSSMERR_TP_NOT_TRUSTED. codesign signs with it regardless, and establishing
# trust would mean a GUI authorization prompt for no benefit — nothing needs to
# *verify* this signature, it only has to stay stable across rebuilds.
identity_hash() {
  security find-identity -p codesigning "$(keychain_path)" 2>/dev/null \
    | grep -F "$IDENTITY_NAME" \
    | head -n 1 \
    | awk '{ print $2 }'
}

# Keep the keychain in the user search list so codesign can find the identity,
# without dropping whatever else is already on the list.
add_to_search_list() {
  target=$(keychain_path)
  current=$(security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"$//')
  if printf '%s\n' "$current" | grep -qF "$KEYCHAIN_NAME"; then
    return 0
  fi
  # shellcheck disable=SC2086
  security list-keychains -d user -s $(printf '%s\n' "$current" | tr '\n' ' ') "$target"
}

EXISTING=$(identity_hash)
if [ -n "$EXISTING" ]; then
  add_to_search_list
  printf '%s\n' "$EXISTING"
  exit 0
fi

log "Creating a local signing identity so Accessibility permission survives rebuilds…"

mkdir -p "$SUPPORT_DIR"
if [ ! -f "$PASSWORD_FILE" ]; then
  umask 077
  LC_ALL=C tr -dc 'A-Za-z0-9' < /dev/urandom | head -c 32 > "$PASSWORD_FILE"
fi
chmod 600 "$PASSWORD_FILE"
PASSWORD=$(cat "$PASSWORD_FILE")

WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/dooni-signing.XXXXXX")
trap 'rm -rf "$WORK_DIR"' EXIT HUP INT TERM

# A config file is used instead of -addext because the openssl shipped with
# macOS does not reliably support -addext.
cat > "$WORK_DIR/openssl.cnf" <<'CONF'
[req]
distinguished_name = dn
x509_extensions = v3
prompt = no

[dn]
CN = dooni local signing

[v3]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
subjectKeyIdentifier = hash
CONF

# Use the system openssl explicitly. A newer openssl earlier on PATH (Homebrew
# or conda) defaults to a PKCS#12 MAC that macOS's security tool rejects with
# "MAC verification failed", and the legacy PBE flags keep the bundle readable
# whichever version ends up running.
OPENSSL=/usr/bin/openssl

"$OPENSSL" req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -config "$WORK_DIR/openssl.cnf" -extensions v3 \
  -keyout "$WORK_DIR/key.pem" -out "$WORK_DIR/cert.pem" >/dev/null 2>&1

"$OPENSSL" pkcs12 -export \
  -inkey "$WORK_DIR/key.pem" -in "$WORK_DIR/cert.pem" \
  -name "$IDENTITY_NAME" -out "$WORK_DIR/identity.p12" \
  -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 \
  -passout "pass:$PASSWORD" >/dev/null 2>&1

if [ ! -f "$(keychain_path)" ]; then
  security create-keychain -p "$PASSWORD" "$KEYCHAIN_NAME"
fi
KEYCHAIN=$(keychain_path)

# No lock timeout, so later rebuilds can sign without a password prompt.
security set-keychain-settings "$KEYCHAIN"
security unlock-keychain -p "$PASSWORD" "$KEYCHAIN"
security import "$WORK_DIR/identity.p12" -k "$KEYCHAIN" -P "$PASSWORD" \
  -T /usr/bin/codesign >/dev/null
# Lets codesign use the key without raising a GUI authorization prompt.
security set-key-partition-list -S apple-tool:,apple:,codesign: \
  -s -k "$PASSWORD" "$KEYCHAIN" >/dev/null 2>&1 || true

add_to_search_list

CREATED=$(identity_hash)
if [ -z "$CREATED" ]; then
  log "dooni: could not create a local signing identity; falling back to ad-hoc signing."
  exit 1
fi

printf '%s\n' "$CREATED"
