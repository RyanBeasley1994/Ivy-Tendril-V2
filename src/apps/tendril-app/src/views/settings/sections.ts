import {
  Activity,
  Bell,
  Bot,
  Cog,
  Cpu,
  Feather,
  FileText,
  Folder,
  FolderGit2,
  GitBranch,
  KeyRound,
  Lock,
  Send,
  Sun,
  Wand,
} from "lucide-react";
import type React from "react";
import type { TFunction } from "../../i18n";

/**
 * `Apps/Settings/SettingsApp.cs`'s section tags, verbatim. They are also the suffix an `activeNav`
 * of `settings:<tag>` carries, so a deep link and V1's `SettingsAppArgs.Section` spell a section the
 * same way.
 */
export const SettingsTag = {
  CodingAgent: "coding-agent",
  Plans: "plans",
  Appearance: "appearance",
  Projects: "projects",
  Vault: "vault",
  Promptwares: "promptwares",
  Levels: "levels",
  Git: "git",
  Notifications: "notifications",
  Security: "security",
  /** V1 keeps a separate tag that selects the same row and the same view as `security`. */
  Tunnel: "tunnel",
  Advanced: "advanced",
  /** Which agent and model each role runs on, the rate-limit fallbacks and the model catalogue. */
  Models: "models",
  Telegram: "telegram",
  ApiKeys: "api-keys",
  /** The daemon itself: its connection, its service and its config file. */
  System: "system",
} as const;

export type SettingsTagValue = (typeof SettingsTag)[keyof typeof SettingsTag];

/** `selected.Set($"project:{i}")`: a project row's tag is its index in `config.Settings.Projects`. */
export const PROJECT_TAG_PREFIX = "project:";

export const projectTag = (index: number): string => `${PROJECT_TAG_PREFIX}${index}`;

/**
 * The index a `project:<n>` tag names, or `null` for anything else. V1 parses the same suffix with
 * `int.TryParse` and falls back to the first project when it does not resolve.
 */
export const projectIndexOf = (tag: string): number | null => {
  if (!tag.startsWith(PROJECT_TAG_PREFIX)) return null;
  const parsed = Number.parseInt(tag.slice(PROJECT_TAG_PREFIX.length), 10);
  return Number.isInteger(parsed) && parsed >= 0 ? parsed : null;
};

export interface SettingsSection {
  label: string;
  tag: SettingsTagValue;
  icon: React.ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
  /** `Projects` is the one expandable row, with a sub-item per project plus "Add Project". */
  expandable?: boolean;
  /** The heading this row sits under in the sidebar. */
  group: string;
}

/**
 * The settings pages, grouped by what they are for rather than in the order they were added:
 *
 * - **Agents and models**: the coding agents installed, which one each role runs on, and the workflow
 *   agents that run on them.
 * - **Work**: the projects, how plans are written and sized, branches, and the team vault.
 * - **Reaching you**: how the daemon gets your attention, and how you answer from your phone.
 * - **Access**: who and what may reach the daemon from outside.
 * - **This app**: how it looks, its limits, and the daemon underneath.
 *
 * Each page holds one concern. The tags are deep links (`settings:<tag>`), so the ones that existed
 * keep their spelling; `levels` now opens Plans, which is where levels live, and `tunnel` still opens
 * remote access.
 *
 * `sections` also drives the mobile picker, which is why one list produces both presentations. The
 * labels are translated with the `t` it is handed, so the list is rebuilt when the language changes.
 */
export function settingsSections(isBeta: boolean, t: TFunction<"settings">): SettingsSection[] {
  const agents = t("groups.agents");
  const work = t("groups.work");
  const reach = t("groups.reach");
  const access = t("groups.access");
  const app = t("groups.app");
  return [
    { label: t("sections.codingAgent"), tag: SettingsTag.CodingAgent, icon: Bot, group: agents },
    { label: t("sections.models"), tag: SettingsTag.Models, icon: Cpu, group: agents },
    // "Workflow Agents" rather than plain "Agents": the row above is the external CLI a workflow
    // agent *runs on*, and two rows both reading "Agent" would be two things under one word.
    { label: t("sections.promptwares"), tag: SettingsTag.Promptwares, icon: Wand, group: agents },

    { label: t("sections.projects"), tag: SettingsTag.Projects, icon: Folder, expandable: true, group: work },
    { label: t("sections.plans"), tag: SettingsTag.Plans, icon: Feather, group: work },
    { label: t("sections.git"), tag: SettingsTag.Git, icon: GitBranch, group: work },
    // The vault row is gated, not always present.
    ...(isBeta
      ? [{ label: t("sections.vault"), tag: SettingsTag.Vault, icon: FolderGit2, group: work } as SettingsSection]
      : []),

    { label: t("sections.notifications"), tag: SettingsTag.Notifications, icon: Bell, group: reach },
    { label: t("sections.telegram"), tag: SettingsTag.Telegram, icon: Send, group: reach },

    { label: t("sections.security"), tag: SettingsTag.Security, icon: Lock, group: access },
    { label: t("sections.apiKeys"), tag: SettingsTag.ApiKeys, icon: KeyRound, group: access },

    { label: t("sections.appearance"), tag: SettingsTag.Appearance, icon: Sun, group: app },
    { label: t("sections.advanced"), tag: SettingsTag.Advanced, icon: Cog, group: app },
    { label: t("sections.system"), tag: SettingsTag.System, icon: Activity, group: app },
  ];
}

/** The icon on the "Open config.yaml" action row (`Icons.FileText`). */
export const OpenConfigIcon = FileText;

/**
 * `SettingsApp.Build`'s `currentLabel`, used for the mobile picker's trigger. A `project:<n>` tag is
 * not in `sections`, so V1 falls back to "Configuration"; that fallback is kept, but a selected
 * project supplies its own name because otherwise the mobile header would read "Configuration" for
 * every project.
 */
export function sectionLabel(
  tag: string,
  sections: SettingsSection[],
  projectNames: string[],
  t: TFunction<"settings">,
): string {
  const index = projectIndexOf(tag);
  if (index !== null) return projectNames[index] ?? t("sections.fallback");
  return sections.find((section) => section.tag === tag)?.label ?? t("sections.fallback");
}
