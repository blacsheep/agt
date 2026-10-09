# Shared resource management

The seven definition resources (Agent, Model, Provider, Prompt, Skill, MCP,
Harness) share `agt_service::Resources`. CLI commands decode flags/files/stdin
and project the results into their existing output shapes. Framework tools and
Web requests use the same service after their identity and capability checks.
Web continues to call Rust through the CLI; no second resource implementation
lives in Node.

`resources/mod.rs` routes operations; the seven resource modules own their
write contracts. Existing domain crates retain configuration validation and
Store persistence. Skill document replacement is now a SkillStore operation.
Category mutations and dependency-aware deletion use the same management lock.

## Update contract

`resource_write` keeps its existing `kind/reference/create/data` interface:

- Omitted optional data fields retain their previous values.
- Agent `data.skills/mcps/builtin_tools` replace entire collections.
- On updates, `add` and `remove` accept those three Agent collections.
  References are normalized, additions deduplicate, and missing removals fail.
- Replacement/clearing and collection operations cannot target the same field.
  Adding and removing the same normalized item is also rejected.
- `clear` supports Agent description and collections, Model max_tokens and
  reasoning_effort, and Harness model and reasoning_effort.
- Other resources reject collection operations rather than ignoring them.
- Existing Model clear_* flags retain their previous behavior.
- Creation rejects all update modifiers.

Example:

```json
{
  "kind": "agent",
  "reference": "default/searcher",
  "create": false,
  "data": {},
  "add": {"mcps": ["default/search"]}
}
```

Read/write results include an opaque content `revision`. Updates optionally
accept `expected_revision`; a mismatch returns `conflict` before changing
anything. Web Agent/Harness editors send the version they read. Legacy clients
without a version retain their existing last-write semantics.

The stable home-level `.resources.lock` serializes management reads, writes,
category changes and deletion execution. Revision comparison, patching and
commit happen under that lock. Existing atomic writes and MCP staging remain.
Content revisions include unmasked persisted data and Skill ancillary files;
only the hash is returned, never credentials. Direct external file edits do not
participate in the lock; revisions detect edits observed before the check.
Low-level domain Stores remain persistence APIs, not an alternative management
entry point.

Deletion retains its existing dependency graph, explicit plan confirmation,
bound-Agent protection and identity cleanup. Resource files and IM/binding
databases are still separate transaction domains.

## Runtime activation

CRUD commits configuration. Native Runtime separately refreshes capabilities
after successful self MCP/Skill writes, at a tool-batch boundary. It reports
applied or failed independently from the write result. Other running Agents,
Model/Provider/base Prompt changes and Harness sessions keep their existing
activation semantics. See [MCP contracts](mcp-tool-contracts.md).

Import formats, remote model discovery and MCP probes retain their resource
specific operations. Saving a definition does not probe a server, install
dependencies or enable an Agent identity.
