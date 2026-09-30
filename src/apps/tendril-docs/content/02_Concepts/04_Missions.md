---
title: Missions
description: >-
  A mission hands Tendril a whole feature. An AI orchestrator breaks it into milestones, runs and
  judges each one, validates the result, and delivers one pull request.
icon: Rocket
searchHints:
  - mission
  - milestone
  - orchestrator
  - epic
  - planner
  - worker
  - judge
  - validator
  - harness
  - autonomous
---

# Missions

A [plan](01_Plans.md) is one change: one revision, one `ExecutePlan` run, one review, one pull
request. A **mission** is a whole feature made of many such changes. You state the goal once, approve
the milestone plan once, and an AI orchestrator runs the rest on its own.

## Plan or mission?

|                      | Plan                                        | Mission                                                                 |
| -------------------- | ------------------------------------------- | ----------------------------------------------------------------------- |
| Size                 | One reviewable change                       | A feature or epic: several changes that build on each other             |
| Who breaks it down   | You (or `SplitPlan`)                        | The planner, into 2–8 milestones with acceptance criteria                |
| Your decisions       | Approve the plan, review the result, create the PR | Approve the milestones once; review the one integration plan at the end |
| Who checks the work  | Verifications, then you                     | Verifications, then the judge per milestone, then the validator overall  |
| When something fails | The plan goes to Failed; you retry          | The judge retries or re-plans; the mission pauses only when it cannot recover |
| Git                  | One branch per plan                         | One mission branch; every accepted milestone lands on it                 |
| Result               | One PR                                      | One PR, from the integration plan                                        |

Use a plan when you can describe the change in one go. Use a mission when you would otherwise write
several dependent plans and babysit them in order.

## How a mission runs

1. **Plan.** The planner researches the goal in the project's repos and writes the milestones. Each
   one has a spec concrete enough for `ExecutePlan` and acceptance criteria that can be checked
   against the diff.
2. **Approve.** The mission waits for you. This is the only stop.
3. **Run and judge**, one milestone at a time:
   - Tendril creates the milestone's plan and runs `ExecutePlan` on a worktree cut from the mission
     branch, so each milestone builds on the ones before it.
   - The judge reviews the diff and the verification reports against the acceptance criteria and
     decides: **accept** (the mission branch fast-forwards to the milestone), **retry** (`RetryPlan`
     with specific feedback), **re-plan** (the remaining milestones are rewritten), or **fail** (the
     mission pauses for you).
4. **Validate.** When every milestone has passed, the validator runs all of the project's
   verifications on the mission branch and checks the result against the goal. Gaps become fix-up
   milestones, which go through step 3 like any other.
5. **Review.** The mission's **integration plan** (level Epic) moves to Review with every milestone's
   commits. You create its pull request the same way as for any plan.

The driver that moves a mission along keeps all of its state in `mission.yaml`, so a mission survives
agent crashes, job timeouts and daemon restarts. The AI makes the decisions; the driver only carries
them out.

## Harnesses per role

Each role can run on a different coding agent, with an optional model and effort:

| Role          | Runs                                   |
| ------------- | -------------------------------------- |
| **Planner**   | The Plan phase                         |
| **Worker**    | `ExecutePlan` and `RetryPlan` per milestone |
| **Judge**     | The review of each milestone           |
| **Validator** | The final validation                   |

Pick them in the Create New Plan dialog (switch it to **Mission**), change them on the Missions page
while the mission runs (changes apply from the next job), or pass them to the CLI as
`agent[:model[:effort]]`. A role left unset runs on your default coding agent. Settings → Coding Agent
→ *Harnesses in pickers* chooses which agents the pickers offer.

## Limits

A mission pauses rather than running away:

- **Attempts per milestone** (default 3): executions allowed before the mission pauses.
- **Re-plans** (default 2): how often the milestone list may be rewritten.
- **Budget** (optional): total spend in USD after which the mission pauses.

Resume a paused mission once you have dealt with the reason; the open milestone gets a fresh attempt
budget. Raise the budget with `tendril mission budget` first if that was the reason.

## CLI

```bash
# Start a mission (the planner starts immediately if the daemon is running)
tendril mission create "Add SSO login" --project=MyApp \
  --goal="Users can sign in with our Okta tenant; keep password login as a fallback." \
  --worker=codex:gpt-5.6-sol:high --judge=claude --max-cost=40

tendril mission list
tendril mission get 1              # goal, milestones, agents, budget, log
tendril mission approve 1          # the one approval
tendril mission pause 1 --reason="Waiting on API keys"
tendril mission resume 1
tendril mission agents 1 --validator=gemini
tendril mission budget 1 --max-cost=60
tendril mission cancel 1
```

`tendril mission set-milestones` and `tendril mission decide` are what the orchestrator itself uses
to record its work; you do not normally run them by hand.
