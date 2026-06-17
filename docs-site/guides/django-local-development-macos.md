---
title: "Django Local Development on macOS with HTTPS"
description: Run Django locally on macOS with a real HTTPS .test hostname, managed DNS, and bundled PostgreSQL and Redis. Open source, container-free, no hosts-file edits.
---

# Django local development on macOS with HTTPS

Point PortBay at a Django folder and it detects the project, runs the development server, and serves it at a real `https://name.test` hostname through Caddy with a trusted local certificate. No container build, no `/etc/hosts` editing, no `runserver_plus` certificate workarounds. The services Django leans on, PostgreSQL and Redis, are bundled and supervised, one click to start with connection details ready for your `settings.py`. Run a Django API beside a Next.js front end on its own hostname in the same session. PortBay is open source under AGPL-3.0, runs natively, and holds idle RAM under 80 MB.

## How PortBay detects a Django project

It recognizes the Python project from `manage.py` and the usual markers, then fills in the start command and port. Pin the Python version per project. Details: [Languages and Runtimes](/guides/languages) and [Add a Project](/getting-started/add-project).

## Real HTTPS for Django

A genuine `https://app.test` certificate makes secure cookies, CSRF over HTTPS, and OAuth callbacks behave the way they will in production, with no browser warning to click through. See [Caddy and HTTPS](/guides/caddy-https).

## PostgreSQL and Redis, supervised

Start a bundled PostgreSQL or Redis from PortBay and point `DATABASES` and your cache at the supplied host and port. Nothing installed by hand, nothing left running after you quit. See [Databases](/guides/databases).

## Start

Install PortBay via [DMG or Homebrew](/getting-started/install), point it at your Django folder, and press Play. Hand the stack to an AI agent through the [task board](/guides/run-ai-coding-agents-locally), and see the [comparisons](/comparisons/) for how PortBay differs from container tools.

_Last updated: 2026-06-15._
