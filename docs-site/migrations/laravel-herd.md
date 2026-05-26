---
title: Migrate from Laravel Herd to PortBay
description: Move your Laravel Herd projects to PortBay — capture hostnames, PHP versions, certificates, and worker commands, then map them to PortBay's PHP registry format.
---

# Migrating From Laravel Herd

This is a release placeholder. Full Herd import documentation will land after the import card is complete.

## What To Capture

- Project folder.
- Herd hostname.
- PHP version.
- Valet/Herd secured certificate state.
- Queue, scheduler, or worker commands.
- Mail and database expectations.

## PortBay Target

Laravel projects normally map to:

```json
{
  "type": "php",
  "document_root": "public",
  "services": ["caddy", "php-fpm"],
  "https": true
}
```

Add worker processes as separate projects or future service hooks when the workflow is ready.
