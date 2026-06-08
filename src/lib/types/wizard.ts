/**
 * Wire shape of `commands::dto::DetectedProject`.
 */
import type {
  LanguageIntelligenceCapability,
  MobileRunConfig,
  ProjectType,
  WebServer,
} from "./projects";

export interface DetectedProject {
  kind: ProjectType;
  suggestedId: string;
  suggestedName: string;
  suggestedHostname: string;
  suggestedPort: number | null;
  suggestedStartCommand?: string;
  suggestedDocumentRoot?: string;
  suggestedPhpVersion?: string;
  suggestedWebServer?: WebServer;
  suggestedMobileRun?: MobileRunConfig | null;
  languageIntelligence: LanguageIntelligenceCapability[];
}
