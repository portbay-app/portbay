/**
 * Shared TLS-source draft logic.
 *
 * A "certificate" in PortBay is the TLS configuration on a project
 * (`ProjectView.domain` + the on-disk cert). Both the Certificates page and the
 * Add Certificate panel edit the same SSL fields, so the draft shape and the
 * `DomainConfig` round-trip live here once instead of being copied per surface.
 *
 * The draft is the SSL subset only — routing fields (notes, pathPrefix,
 * resolverMode, exposeWhenRunning) are preserved from the project's existing
 * `domain` when building the patch, never edited here.
 */

import {
  defaultAcmeConfig,
  defaultDomainConfig,
  type AcmeDnsProvider,
  type AcmeEnvironment,
  type AcmeIssuer,
  type AcmeKeyType,
  type DomainConfig,
  type SslMode,
} from "$lib/types/projects";

export interface TlsDraft {
  https: boolean;
  autoManageCert: boolean;
  sslMode: SslMode;
  customCertPath: string;
  customKeyPath: string;
  acmeIssuer: AcmeIssuer;
  acmeEnvironment: AcmeEnvironment;
  acmeEmail: string;
  acmeKeyType: AcmeKeyType;
  acmeEabKeyId: string;
  acmeEabHmacKey: string;
  acmeZerosslApiKey: string;
  acmeDnsProvider: AcmeDnsProvider;
  acmeDnsApiToken: string;
  acmeForceRequest: boolean;
  acmeDebug: boolean;
  includeWildcardSubdomains: boolean;
}

/** A fresh draft for a project that has no cert yet (HTTPS on, automatic local). */
export function emptyTlsDraft(): TlsDraft {
  return tlsDraftFromDomain(null, true);
}

/** Seed a draft from a project's `domain` config (or defaults when absent). */
export function tlsDraftFromDomain(
  domain: DomainConfig | null | undefined,
  https: boolean,
): TlsDraft {
  const d = domain ?? defaultDomainConfig();
  const acme = d.acme ?? defaultAcmeConfig();
  return {
    https,
    autoManageCert: d.autoManageCert ?? true,
    sslMode: d.sslMode ?? "automatic_local",
    customCertPath: d.customCertPath ?? "",
    customKeyPath: d.customKeyPath ?? "",
    acmeIssuer: acme.issuer ?? "lets_encrypt",
    acmeEnvironment: acme.environment ?? "production",
    acmeEmail: acme.email ?? "",
    acmeKeyType: acme.keyType ?? "p384",
    acmeEabKeyId: acme.eabKeyId ?? "",
    acmeEabHmacKey: acme.eabHmacKey ?? "",
    acmeZerosslApiKey: acme.zerosslApiKey ?? "",
    acmeDnsProvider: acme.dnsProvider ?? "none",
    acmeDnsApiToken: acme.dnsApiToken ?? "",
    acmeForceRequest: acme.forceRequest ?? false,
    acmeDebug: acme.debug ?? false,
    includeWildcardSubdomains: d.includeWildcardSubdomains ?? false,
  };
}

/**
 * Build the `DomainConfig` patch from the draft, preserving the project's
 * existing routing fields (notes/pathPrefix/resolverMode/exposeWhenRunning).
 * Mirrors the cert page's `buildDomain`.
 */
export function buildDomainFromTls(
  base: DomainConfig | null | undefined,
  d: TlsDraft,
): DomainConfig {
  const current = base ?? defaultDomainConfig();
  return {
    ...current,
    autoManageCert:
      d.https && d.sslMode === "automatic_local" ? d.autoManageCert : false,
    sslMode: d.sslMode,
    customCertPath: d.customCertPath.trim() ? d.customCertPath.trim() : null,
    customKeyPath: d.customKeyPath.trim() ? d.customKeyPath.trim() : null,
    acme:
      d.sslMode === "public_acme"
        ? {
            issuer: d.acmeIssuer,
            environment: d.acmeEnvironment,
            email: d.acmeEmail.trim() ? d.acmeEmail.trim() : null,
            keyType: d.acmeKeyType,
            eabKeyId: d.acmeEabKeyId.trim() ? d.acmeEabKeyId.trim() : null,
            eabHmacKey: d.acmeEabHmacKey.trim() ? d.acmeEabHmacKey.trim() : null,
            zerosslApiKey: d.acmeZerosslApiKey.trim()
              ? d.acmeZerosslApiKey.trim()
              : null,
            dnsProvider: d.acmeDnsProvider,
            dnsApiToken: d.acmeDnsApiToken.trim()
              ? d.acmeDnsApiToken.trim()
              : null,
            forceRequest: d.acmeForceRequest,
            debug: d.acmeDebug,
          }
        : null,
    includeWildcardSubdomains: d.includeWildcardSubdomains,
  };
}

export const sslModeOptions: { value: SslMode; label: string }[] = [
  { value: "automatic_local", label: "Automatic local HTTPS" },
  { value: "custom_certificate", label: "Custom certificate" },
  { value: "self_signed", label: "Self-signed fallback" },
  { value: "public_acme", label: "Public ACME / AutoSSL" },
];

export const acmeIssuerOptions: { value: AcmeIssuer; label: string }[] = [
  { value: "lets_encrypt", label: "Let's Encrypt" },
  { value: "zero_ssl", label: "ZeroSSL" },
  { value: "google_trust_services", label: "Google Trust Services" },
];
export const acmeEnvironmentOptions: { value: AcmeEnvironment; label: string }[] =
  [
    { value: "production", label: "Production" },
    { value: "staging", label: "Staging" },
  ];
export const acmeKeyTypeOptions: { value: AcmeKeyType; label: string }[] = [
  { value: "p384", label: "ECC P-384" },
  { value: "p256", label: "ECC P-256" },
  { value: "rsa2048", label: "RSA 2048" },
  { value: "rsa4096", label: "RSA 4096" },
];
export const dnsProviderOptions: { value: AcmeDnsProvider; label: string }[] = [
  { value: "none", label: "HTTP/TLS challenge" },
  { value: "cloudflare", label: "Cloudflare" },
];
