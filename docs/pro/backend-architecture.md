# PortBay Cloud — Backend Architecture (moved)

The PortBay Cloud backend architecture lives in the **private `portbay-cloud`
repository** (`docs/backend-architecture.md`), not here.

It describes server-side implementation — the hosting stack, data model, and
internal endpoints — which is proprietary and intentionally kept out of the
public AGPL repository. See [repo boundaries](../architecture/repo-boundaries.md).

What *is* public:

- The client ↔ server **entitlement contract**: [entitlements.md](./entitlements.md)
- How the Community app integrates with Cloud without exposing private code:
  [cloud integration](../architecture/cloud-integration.md)
