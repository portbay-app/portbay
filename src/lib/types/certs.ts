/**
 * Cert metadata surfaced by `cert_info(id)`. Mirrors
 * `commands::certs::CertInfo` on the Rust side.
 */
export interface CertInfo {
  projectId: string;
  certificatePath: string;
  keyPath: string;
  issuedAt: string | null;
  expiresAt: string | null;
  daysUntilExpiry: number | null;
  sans: string[];
}
