# Local web console

Branch: codex/local-web-console, based on c24fcb3. No existing frontend/Trellis
workflow is present. Implement a dependency-free Node HTTP adapter and static UI;
reuse the built agt CLI and its service, not a parallel business implementation.

Scope: Chinese local operator console, Agent setup (model/prompt/mounts/binding),
private/group chat, explicit mentions, read-only identity observation, bounded
worker batches, run/call diagnostics, reusable resource forms. No game-specific
features or autonomous scheduler. Existing resources are never silently rewritten.

Security: IPv4 loopback only, exact Host/Origin checks, random per-launch token,
JSON API with auth header, no shell command construction from browser input,
allowlisted operations, explicit paid execution and mounted MCP opt-in. API is a
trusted host administration surface, not a public multi-tenant server.

Verification: Node unit/HTTP tests with stub CLI and real CLI integration; browser
visual/interaction inspection in isolated home, no paid model calls; workspace
regression tests and clean Git commits. UI must show errors, empty states, loading,
partial setup progress, and distinguish message delivery from model completion.

## Delivered and verified

- Dependency-free local HTTP adapter and responsive Chinese console, with launch
  and manual acceptance instructions in web/README.md. No Rust business logic changes.
- Independent read-only review: retained explicit trusted-host observer privilege;
  fixed stale message/list responses and modal response races. Partial Agent setup
  locks persisted fields; scoped Skill references preserve their package identity.
- Browser, isolated home: create model and Agent/binding, private and group chat,
  send messages, safely render literal HTML, confirm a manual-policy worker batch,
  inspect skipped delivery, switch read-only observer. No paid model requests.
- Layout checks at 1440, 1024 and 390 pixels; no page horizontal overflow.
  Runtime observations remain available below chat on smaller screens.
- 4 Node HTTP/integration tests passed (auth/origin/allowlist, bounded worker,
  malformed inputs/redaction, real CLI with scoped Skill/identity visibility).
- 245 Rust workspace tests passed offline; cargo fmt check, Clippy with warnings
  denied and JavaScript syntax checks passed.

Intentional limits: explicit batches, no continuous scheduler; resource creation
and binding changes in UI, advanced edits via CLI; no public multi-user access.
Live paid-model acceptance through the UI remains for the user.
