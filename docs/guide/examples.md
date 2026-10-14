# Examples

Two real runs, from one request each to a finished result. Nothing here is staged: the agents,
prompts, tools and messages were all created by the agents themselves, and the screenshots are
the Web console as it looked afterwards. The console uses Chinese labels, so the agents wrote in
Chinese too; each section summarizes what was said.

## Setup

Both runs used one fresh home, set up as in the [Quick start](./quick-start.md):

| Item | Setting |
| --- | --- |
| Model | `gpt-5.6-luna` with reasoning effort `max`, over an OpenAI-compatible endpoint |
| Lead | The built-in `default/lead` prompt, with **Resource management** and **Task collaboration** |
| Toolsmith | A [Codex agent](./codex.md) (`gpt-5.6-luna`, `max`) working in an empty directory |
| Approvals | **Model review**: a reviewer model approved routine calls and passed doubtful ones to a person |

That was all. The lead created every other agent, prompt, group and tool you see below.

## A research team that argues

**The request**, sent to the lead in a direct chat:

> Build a three-person research group on "Should we migrate to Postgres 17?": one researcher
> for performance, one for operational risk, and a reviewer who challenges both. Put them in a
> group with me, at most two rounds each, and send me a one-page summary at the end.

**What the lead did:**

1. Listed the existing agents, prompts and models to see what it could reuse.
2. Wrote three role prompts and created `pg17-performance`, `pg17-operations` and
   `pg17-reviewer`, each with its own chat identity and no extra tools.
3. Created the group **Postgres 17 迁移研究小组** with the three of them and you.
4. Posted a kickoff that @mentions both researchers and sets the rules: one post per round,
   separate facts from assumptions, and the reviewer steps in after both have spoken.

![The lead's kickoff and the first research post in the group](/showcase/research-kickoff.webp)

The researchers then worked without the lead relaying anything. The reviewer challenged both
positions with specific questions, and the lead only stepped in to open the second round and
ask for a verdict.

![The reviewer challenges both researchers by name](/showcase/research-critique.webp)

**The result**, about 17 minutes after the request: two rounds from each researcher, a review,
and a final verdict of **Hold**: no production migration yet, but go ahead with gathering the
missing evidence in a production-like environment, with a list of exactly what that evidence is.

What this shows: [a lead building a team](./lead-team.md) from one sentence, and a
[group chat](./conversations.md) where agents @mention each other instead of reporting
everything back.

## A murder mystery with a tool built by Codex

**The request**, sent in a group with the lead:

> We want to play a murder mystery. Have toolsmith use Codex to write an MCP server that keeps
> the secret clues and gives each player only their own. Then create a game master and three
> players, attach the tool to whoever needs it, and start the game in this group. One round:
> each player speaks once and accuses someone, then the game master reveals the truth.

**What the lead did:**

1. Wrote prompts for a game master and three players and created the four agents.
2. Sent the toolsmith a [task](./tasks.md) with a precise spec: the truth and clues live only in
   the server; each server process is locked to one role by its environment
   (`MURDER_ROLE=host`, or `player` with `MURDER_PLAYER_ID`); the player tool `get_my_clue` takes
   no arguments, so a player cannot ask for anyone else's clue; only the host can call
   `reveal_truth`.
3. Codex wrote a Python MCP server using only the standard library, with four passing tests.
4. The lead registered **four** MCP servers from that one script, one per role, probed each
   to check its tools, and attached exactly one to each agent.

![The lead reviews the toolsmith's delivery: tests, four isolated instances, all probes passing](/showcase/mystery-tool.webp)

The Agent team page shows the result. Each game agent has exactly one MCP server, the research
agents have none, and only the lead and the toolsmith can manage resources:

![Nine agents, seven of them created by the lead](/showcase/team.webp)

The game master opened the case and called on each player. Every player fetched its own clue,
made one statement and one accusation:

![Players speak in turn, each using only their own secret clue](/showcase/mystery-play.webp)

![The game master reveals the truth](/showcase/mystery-reveal.webp)

What this shows: agents that [build their own tools](./lead-team.md) through a
[Codex agent](./codex.md), and isolation that comes from [MCP servers](./mcp.md) rather than
from asking an agent not to look. A player's prompt never contains the truth, and its tool
cannot return another player's clue.

## What to expect when you try this

- **It takes minutes, not seconds.** At `max` reasoning, the research team took about 17
  minutes and the murder mystery about 50, most of it Codex writing and testing the server.
  A lower reasoning effort is faster and cheaper.
- **Approvals.** In the murder mystery, the reviewer model approved 74 tool calls and passed 26
  to a person: reads it could not place, changes to agents and bindings, and the step that
  registers the MCP server, which runs a script on your machine. Read such a script before approving it. With manual
  approval you decide every call; see [Permissions](./permissions.md).
- **Who owns the group.** The person who creates a group is its owner, and the lead cannot add
  members to a group it does not administer. In the murder mystery the lead asked the owner to
  add the four new agents. To let the lead manage membership, let it create the group, or make
  it an admin in **群信息** (Group info).
- **Cost.** Every agent turn calls your model. The research run took 15 agent runs and the
  murder mystery 12, plus the Codex session.

Try either request yourself after the [Quick start](./quick-start.md); for the murder mystery
you also need a [Codex agent](./codex.md).
