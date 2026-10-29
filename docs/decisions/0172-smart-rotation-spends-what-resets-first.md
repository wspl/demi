---
status: accepted
---

# Smart rotation spends the quota that resets first

## Problem

Alex pays for three Claude subscriptions and lets Balanced rotation pick an
account for each agent. Account A has 30% of its weekly limit left and
resets tomorrow; account B has 90% left and resets in six days. Both 5-hour
windows are unused. Balanced takes the room of the tightest window, which is
the unused 5-hour window's 1.0 on both, so the two tie and the first in the
list wins. When B is listed first, agents work on B, and A's 30% is gone at
its reset: Alex paid for work that never ran, and later in the week, when B
runs out, the agents wait.

A second rule wastes quota the same way. An account whose window is more
than 90% used is left out while another has room. Account A at 93% of its
week, three hours before the reset, takes no agent, and its last 7% is lost.

Alex's goal is to get the most work out of what the subscriptions cost:
quota left at a reset is money spent on nothing.

## Decision

The rotation Balanced becomes **Smart**: each agent takes the account whose
weekly quota would be lost soonest if it waited, as long as its short window
can take the agent.

```text
Rotation
  ( ) Manual     Every agent uses the active account.
  ( ) In Order   Each agent takes the first account that has room.
  (•) Smart      Each agent takes the account whose quota resets soonest
                 with room left.
```

A choice, for an agent of model `M`, among the accounts of the entry:

1. Leave out an account that is used up: marked used up, or with a window
   that applies to `M` at 100%.
2. Leave out an account whose **short window**, any window shorter than its
   longest, is 80% used or more, while another account passes this step.
3. Rank the rest by the **urgency** of the longest window that applies to
   `M`, the tightest when several have the longest period: the share left
   divided by the share of its period left,
   `(1 − used) / min(1, until reset ÷ period)`. A window not started, past
   its reset, or without a reset time or a period counts 1, as does an
   account with no snapshot. The most urgent wins.
4. Accounts within 30% of the best urgency are equal; among them, the one
   fewest agents run a turn on wins, then the first in the list.

For example, A at 70% of its week with one day to go has an urgency of
0.3 / (1/7) = 2.1, and B at 10% with six days to go 0.9 / (6/7) = 1.05: the
agent takes A. A's 5-hour window at 85% would leave A out until it resets
or every account is that full.

What stays as it is:

- An agent keeps the account it took, for the prompt cache, and takes another
  only when its account is used up, after a compaction, or when the account
  is removed or the rotation changes. Smart never moves an agent off a usable
  account.
- *Continue with* offers a Manual entry's account by the same rule.
- A choice under Smart reads snapshots older than five minutes again first,
  as Balanced does.
- In Order and Manual do not change.

### Design

- `shared-types`: `Rotation::Balanced` becomes `Rotation::Smart`, serialized
  `smart`. No old name stays.
- `provider-registry` `rotation.rs`: `Room` and `most_room` give way to the
  four steps above; `NEARLY_USED_UP` goes, and `SHORT_WINDOW_FULL = 0.8` and
  `EQUAL_URGENCY = 0.3` say their reasons beside them. `account.chosen` logs
  `mode = "smart"`, the winner's `urgency_pct`, and `reason` one of
  `most_urgent`, `fewest_running_among_equal_urgency` or `all_used_up`.
- Stored data, by migration:
  - Control schema: `providers.rotation` holds `smart` for `balanced`, and its
    check names `manual`, `in_order` and `smart`.
  - Conversation schema: checkpoint state format 10, whose
    `account.rotation` holds `smart` for `balanced` in `nodes.state`.
- `web-ui`: `SettingsRotation` and `ROTATIONS` name `smart`; the option's
  label and hint change in every locale, and its icon is Lucide's
  `Hourglass`, for quota that runs out with time. The gallery's fixtures and
  the error specimen that names Balanced say Smart.
- An older page that sends `balanced` is refused by the contract like any
  unknown value; pages update with the backend.

## Spike

Whether a rule this simple is close to the best any rule could do, and
which of the published rules wastes least, was not known. A simulator,
outside the repository, ran four weeks of invented work against accounts
with a 5-hour window that starts at first use and a weekly window with its
own fixed reset, and counted the work that could not run because no account
had room. Agents kept their account as above, and moving one rewrote its
prompt cache.

The scenarios: 405 combinations of five account sets (three equal, three
resetting at the same moment, two, one large and three standard, a small
and a large one), a week holding 5, 9 or 15 five-hour windows, demand at 0.8,
1.0 and 1.2 times the accounts' weekly total, three shapes of use (working
hours, nearly all day, and a burst of eight agents for six hours), and three
seeds.

Work that could not run, on average, without the burst shape:

| Demand | In Order | Soonest reset | Urgency squared | Balanced | Smart |
| ------ | -------- | ------------- | --------------- | -------- | ----- |
| 0.8    | 1.3%     | 0.9%          | 0.7%            | 0.6%     | 0.6%  |
| 1.0    | 5.7%     | 3.5%          | 3.1%            | 2.7%     | 2.5%  |
| 1.2    | 17.8%    | 13.8%         | 14.3%           | 14.6%    | 13.3% |

- With demand at 1.2 and use all day on three equal accounts, Balanced lost
  10.9% and Smart 7.7%. Under light demand the two differ by a tenth of a
  point.
- The burst shape loses about 11%, 19% and 30% at the three loads under
  every rule: eight agents at once outrun every 5-hour window, which no
  choice of account changes.
- A ceiling: a rollout (Bertsekas) that knew the next seven days of demand
  and played each account forward before every choice lost 0.3 to 1.5 points
  less than Smart, 1 to 2.4 in bursts. A predictive model could recover only
  part of that, so Smart stays a rule.
- The thresholds hardly matter: a short-window cut at 80% or 90%, equal
  urgency within 10% or 30%, and scaling urgency down by the short window's
  use all moved the loss by about a tenth of a point. Without the cut, Smart
  lost more in every load.
- Choosing again when an idle agent's cache has expired, an hour after its
  last request, lost more, not less: agents that resume together crowd onto
  the same account.

## Prior art

- [better-ccflare](https://github.com/tombii/better-ccflare)
  `session-drain-soonest`: keeps sessions on their account, and ranks
  accounts by the earliest weekly reset, for the same reason, quota that
  expires unused. It ignores how much is left and the 5-hour window, which
  forced twice the moves in the simulation.
- [opencode-auth-load-balancer](https://github.com/owjs3901/opencode-auth-load-balancer):
  ranks by `weekly left / (days to reset + 0.25)²`, scaled down by the
  5-hour window's pressure, and moves a session once its 5-hour window
  passes 95%. Smart takes its idea, perishable weekly quota first, with the
  linear form and without the moves, which cost cache rewrites and lost more
  work.
- [maxpool](https://github.com/2solarmax/maxpool): ranks by remaining quota
  over time to reset, among a dozen other weighted terms.
- [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI): round-robin
  and fill-first only; fill-first is In Order.

## Alternatives

- **Keep Balanced and add Smart.** Balanced wins only under light demand,
  by a tenth of a point, so a fourth option would ask a question whose
  answer barely matters.
- **Soonest reset first**, as better-ccflare: simpler, but an account with
  1% left and a reset tomorrow wins over one with 60% left and a reset in two
  days, and agents move more.
- **A learned or predictive model.** Even knowing the future gains little
  over Smart, and a model's choices are harder to explain in the log.
- **Move agents to a more urgent account.** Each move rewrites the prompt
  cache and spent more than it saved.

## Open questions

- The label: Smart, 智能, 智慧, スマート, 스마트, and the hint "Each agent
  takes the account whose quota resets soonest with room left."
