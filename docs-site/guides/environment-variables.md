# Environment Variables

PortBay stores per-project environment variables in the registry and passes them to the project process when Process Compose launches it.

## Registry Shape

```json
{
  "env": {
    "NODE_ENV": "development",
    "API_BASE_URL": "https://api.test"
  }
}
```

## Safe Defaults

- Store non-sensitive values in the registry.
- Keep secrets out of committed `.portbay.json` files.
- Use `.portbay.json` `secrets` entries to name values that an importer must provide.
- Prefer explicit ports and URLs over environment-dependent framework defaults.

## Portable Project Files

Exported `.portbay.json` files support:

| Field | Purpose |
| --- | --- |
| `envTemplate` | Non-sensitive values safe to commit. |
| `secrets` | Names of required secret variables, never their values. |
| `postInstall` | Commands future import flows can offer to run. |

Example:

```json
{
  "version": 1,
  "name": "Marketing Site",
  "type": "next",
  "hostname": "marketing-site.test",
  "port": 3010,
  "https": true,
  "autoStart": false,
  "startCommand": "pnpm dev",
  "envTemplate": {
    "NEXT_PUBLIC_APP_URL": "https://marketing-site.test"
  },
  "secrets": ["STRIPE_SECRET_KEY"]
}
```
