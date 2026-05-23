# Project Groups

Groups cluster projects so they can be operated as a unit. A group is useful when a feature requires a frontend, API, worker, and local service to move together.

## Group Shape

```json
{
  "id": "commerce-stack",
  "name": "Commerce Stack",
  "projects": ["storefront", "checkout-api", "worker"]
}
```

## Actions

| Action | Effect |
| --- | --- |
| Start group | Starts every member project. |
| Stop group | Stops every member project. |
| Restart group | Restarts every member project. |
| Open group | Navigates to the group view. |

## Operating Guidance

- Keep groups small and meaningful.
- Do not use groups as tags; use project `tags` for filtering.
- Stop a group before changing shared ports, services, or hostnames.
- If one member fails, inspect that member’s row and logs before restarting the full group.
