/**
 * DTOs of a project's memory files and its repo-asset import, as the daemon serializes them
 * (`crates/tendril-server/src/routes/projects/memory.rs` and `repo_assets.rs`).
 *
 * `DiscoveredRepoAsset` and `RepoAssetKind` are the component library's (`ImportRepoAssetsDialog`
 * owns the shape it renders); they are re-exported so app code has one import site.
 */
export type { DiscoveredRepoAsset, RepoAssetKind } from "@ivy-interactive/components/dialogs";

/** One row of a project's memory table: `GET /api/projects/:name/memory`. */
export interface ProjectMemoryEntry {
  fileName: string;
  /** The first two non-blank lines, markers trimmed, joined with " — " (V1's snippet). */
  snippet: string;
  sizeBytes: number;
  /** From the memory's frontmatter; a hand-written file is a `note` from `user`. */
  title?: string;
  /** `architecture`, `convention`, `decision`, `gotcha`, `preference` or `note`. */
  kind?: string;
  /** Who saved it: `user`, `plan:00012`, `job:00018`, `mission:00003`, `chat:<id>`. */
  source?: string;
  updated?: string;
  /** Every path it names has gone from the project's repos. */
  stale?: boolean;
}

/** `GET`/`PUT /api/projects/:name/memory/:file`. */
export interface ProjectMemoryFile {
  fileName: string;
  content: string;
}

export interface DockerContainer {
  id: string;
  name: string;
  image: string;
  state: string;
  status: string;
  ports: string;
}

export interface ProjectDocker {
  available: boolean;
  reason?: string;
  containers: DockerContainer[];
}

/** One role's engine: the coding agent, and optionally a model and effort on it. */
export interface EngineChoice {
  agent: string;
  model?: string;
  effort?: string;
}

export type EngineRole = "manager" | "planner" | "worker" | "judge" | "validator";

export type ProjectEngineRoles = Partial<Record<EngineRole, EngineChoice>>;

/** A screenshot or recording a worker attached to a plan to show its change working. */
export interface EvidenceItem {
  kind: "image" | "video";
  /** Absolute, as the daemon's file route wants it. */
  path: string;
  file: string;
  caption: string;
  step: string;
  at?: string;
}

export interface EvidenceGroup {
  milestoneId: string | null;
  title: string;
  planFolder: string;
  items: EvidenceItem[];
  /** Set when the plan belongs to a mission. */
  missionId?: string | null;
  missionTitle?: string | null;
  /** The newest item's time, for ordering. */
  newest?: string;
}

export interface MissionEvidence {
  groups: EvidenceGroup[];
}

/** The engines every project uses unless it sets its own, and the order agents take over when one is rate limited. */
export interface GlobalEngine {
  roles: ProjectEngineRoles;
  fallbacks: EngineChoice[];
  /** Agents sitting out a rate limit right now, and when each comes back. */
  limited?: { agent: string; until: string }[];
}
