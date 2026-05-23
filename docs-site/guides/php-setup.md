# PHP Setup

PHP projects are modeled as `type: "php"` registry entries with optional PHP-specific fields.

## Fields

| Field | Purpose |
| --- | --- |
| `document_root` | Relative web root, commonly `public`. |
| `php_version` | Version label used to bind the PHP-FPM service. |
| `services` | Shared services required by the project, such as Caddy or PHP-FPM. |
| `env` | Runtime variables passed to project processes where applicable. |

## Laravel Example

```json
{
  "id": "billing-api",
  "name": "Billing API",
  "path": "/Users/you/Projects/billing-api",
  "type": "php",
  "hostname": "billing-api.test",
  "https": true,
  "services": ["caddy", "php-fpm"],
  "document_root": "public",
  "php_version": "8.3",
  "auto_start": false
}
```

## Xdebug

The command palette includes PHP actions for toggling Xdebug mode on PHP projects. When Xdebug is enabled, expect slower requests and keep it off unless you are actively debugging.

## Checklist

1. Confirm the project has a concrete document root.
2. Confirm Composer dependencies are installed.
3. Confirm the selected PHP version is installed or available through PortBay’s PHP service layer.
4. Confirm the hostname routes through Caddy.
5. Confirm logs show PHP-FPM and Caddy as healthy before debugging app code.
