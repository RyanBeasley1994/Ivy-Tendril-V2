import {
  DEFAULT_VALUE,
  PROFILE_TIERS,
  findEntry,
  initialBaseUrl,
  readAgentEntries,
  readProfiles,
  supportsEffort,
  tierDefaults,
  type ProfileTier,
} from "../views/settings/codingAgents";
import type { TendrilConfig } from "../types/api";

/** The tier a chat runs on until one is picked: `apply_profile`'s own fallback. */
export const DEFAULT_CHAT_PROFILE: ProfileTier = "balanced";

export const isProfileTier = (value: unknown): value is ProfileTier =>
  typeof value === "string" && (PROFILE_TIERS as readonly string[]).includes(value);

/**
 * What one profile launches an agent with. An empty `model` means the profile names none, so the chat
 * falls back to the agent's catalog default; an empty `effort` sends none.
 */
export interface ResolvedProfile {
  tier: ProfileTier;
  model: string;
  effort: string;
}

export type ResolvedProfiles = Record<ProfileTier, ResolvedProfile>;

const isSet = (value: string) => value.trim() !== "" && value.trim().toLowerCase() !== DEFAULT_VALUE;

/**
 * The model and effort each profile gives `agentId`, by the rule workflow agents run on
 * (`resolution.rs`'s `apply_profile`): the `codingAgents[].profiles[]` entry the operator set in
 * Settings > Coding Agent, and where a field is left at `default`, the built-in tier default. So a
 * chat on "Balanced" runs exactly what a balanced plan would.
 */
export function resolveChatProfiles(cfg: TendrilConfig | null, agentId: string): ResolvedProfiles {
  const entries = readAgentEntries(cfg);
  const configured = readProfiles(findEntry(entries, agentId));
  const defaults = tierDefaults(agentId, initialBaseUrl(entries, agentId));
  const effortAllowed = supportsEffort(agentId);

  const pick = (own: string, fallback: string) =>
    isSet(own) ? own.trim() : isSet(fallback) ? fallback.trim() : "";

  return Object.fromEntries(
    PROFILE_TIERS.map((tier) => [
      tier,
      {
        tier,
        model: pick(configured[tier].model, defaults[tier].model),
        effort: effortAllowed ? pick(configured[tier].effort, defaults[tier].effort) : "",
      },
    ]),
  ) as ResolvedProfiles;
}
