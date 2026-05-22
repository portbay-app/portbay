/**
 * Wire shape of `commands::dto::DetectedProject`.
 */
import type { ProjectType } from "./projects";

export interface DetectedProject {
  kind: ProjectType;
  suggestedId: string;
  suggestedName: string;
  suggestedHostname: string;
  suggestedPort: number;
  suggestedStartCommand?: string;
}
