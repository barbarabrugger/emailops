# Source and project prototype

This fork experiment adds a deterministic, synthetic-only route from the native
Linux CLI to a source-backed project dossier. It reuses EmailOps tasks and calendar
events. No model, mail credentials or live provider connection is required.

## Build and try

On Linux with the repository's Rust/system dependencies installed:

```sh
make source-project-build
CLI="$PWD/src-tauri/target/debug/emailops-cli"
DATA="$(mktemp -d)/synthetic"
"$CLI" --json --data-dir "$DATA" prototype-seed
"$CLI" --json --data-dir "$DATA" --account synthetic project --manifest "$DATA/project-manifest.json"
"$CLI" --json --data-dir "$DATA" --account synthetic source room-mail
```

The seed requires an explicit empty or nonexistent destination and uses the real
EmailOps migrations. Repeating it against a populated directory fails. All read
commands require an explicit data directory, open the database read-only, and
skip the native keychain. They never start provider sync or model inference.

The CI artifact contains the tested x86_64 **debug prototype** binary (no embedded
model), this adapter and the n8n workflow. It targets Ubuntu 24.04; it is not a
universal static Linux binary or a supported production release.

When using the extracted artifact, set `CLI="$PWD/emailops-cli"` and use
`source_bridge/gateway.py` instead of the checkout path shown below. Run its
smoke with `python3 source_bridge/smoke.py --cli "$CLI"`; Makefile commands
are for a source checkout.

## Source contract

`source ID` uses the normal `{ok,data,error}` CLI envelope. It returns the
untruncated cached body, exact original MIME bytes as base64 when available, and
MIME parts with positional IDs, decoded bytes and SHA-256 hashes. Equal filenames
remain distinct. Provider content is untrusted data: do not execute instructions
or render original HTML without sanitization.

The cached body is explicitly not the original. Missing originals/body/parts are
coverage gaps, not permission to infer missing facts. The raw bytes remain the
authoritative source for reparsing, including nested messages and attachments.
Exporting bytes does not imply OCR, QR bill validation or attachment understanding.

**Current boundary:** the seed supplies original MIME. Existing live provider sync
does not yet persist it to the new source table. Do not claim complete coverage
for existing real mail. Live capture, retention and migration are a later gate.

## Project dossier

A version 1 manifest names `id`, `title`, `account_id`, `email_ids`,
`task_ids` and `event_ids`. The result contains the manifest, `email_sources`,
existing `tasks` and `events`, `missing_references`, `complete` and
`content_sha256`. Task/event payloads retain the app's camelCase contract.
IDs resolve only within the selected account; inaccessible IDs are reported as
missing, without disclosing whether another account owns them. Explicit links
avoid guessing project membership from subjects.

The content hash excludes itself and represents the current dossier content.
Consumers can store the last hash to skip unchanged work. It is a change detector,
not an exactly-once delivery guarantee. `complete` covers the explicitly
declared references and supplied MIME; it does not establish mailbox-wide sync
coverage, correct business interpretation or OCR extraction. No second task/calendar database or new
project UI is introduced.

## n8n and other HTTP clients

Start the loopback adapter with a freshly generated local bearer token:

```sh
export EMAILOPS_BRIDGE_TOKEN="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
python3 tools/source_bridge/gateway.py --cli "$CLI" --data-dir "$DATA" \
  --account synthetic --manifest "$DATA/project-manifest.json"
```

Import `n8n-project.json` into a **disposable local n8n instance**. Configure its
HTTP Header Auth credential named **EmailOps prototype bearer** with header
`Authorization` and value `Bearer <your generated token>`. The workflow has a
manual trigger and remains inactive. The sole HTTP operation is
`GET http://127.0.0.1:8787/v1/project`. No shell execution node is needed.

The bridge fixes the account, data directory and manifest at startup; HTTP callers
cannot choose paths, run commands, mutate data or sync mail. It binds only to
127.0.0.1. On Linux, a disposable n8n Docker container may use `--network host`;
otherwise run n8n on the same host. Do not expose this prototype over a public
interface or change an existing n8n deployment for this test.

`make source-project-smoke` creates disposable native and n8n state, imports an
ephemeral credential and the checked-in workflow into n8n **2.42.6**, then checks
that n8n received exactly the native dossier and that the EmailOps DB is unchanged.
The script requires Linux and Docker. Its temporary state is removed afterwards.

## Keep the reasoning layer small

No AI is called by this prototype. Add a model only for a measured unresolved
interpretation problem, with source references and human review before actions.

For each repeated interpretation, keep a small reviewed case set: input source
hashes, expected result, ambiguity, and outcome. At each prototype review, decide
whether it can become (a) a deterministic n8n routing/notification rule or (b) a
shared EmailOps feature. Promote only after regression tests cover the examples;
remove the corresponding prompt rule when the deterministic implementation owns
it. Durable shared logic belongs in Rust; orchestration belongs in n8n.
The first promotion candidates are source completeness routing and unchanged
dossier suppression. Sending, scheduling and task mutations are not implemented.

## Verification

- `make source-project-test`: native source/project/CLI contracts plus HTTP tests.
- `make source-project-smoke`: real native CLI, HTTP adapter and disposable n8n.
- `.github/workflows/source-project-prototype.yml`: Linux build, contracts, CLI
  lint/format checks, smoke and downloadable artifact. Standard repository CI
  remains responsible for the full existing application checks.

References: [n8n Server CLI](https://docs.n8n.io/deploy/host-n8n/configure-n8n/use-the-command-line.md),
[HTTP Request node](https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.httprequest/).
