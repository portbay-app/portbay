//! Stack recipes — named blueprints that compose a project's setup in one
//! step (framework, PHP version, document root, HTTPS).
//!
//! An agent maps the user's request ("a Laravel app at blog.test") onto a
//! recipe id and calls `portbay_setup_from_recipe`; PortBay applies the
//! blueprint deterministically. This keeps natural-language understanding on
//! the agent (where it's strong) and composition on PortBay (where it's
//! reliable) — no server-side model needed.
//!
//! Recipes whose stack needs a database or local mail catcher declare it via
//! [`Recipe::needs_database`] / [`Recipe::needs_mail`]. Automatic provisioning
//! of those services is a follow-on; today such a recipe registers the project
//! and reports the recommendation as a warning rather than half-wiring it.

use crate::registry::ProjectType;

/// A named stack blueprint. The fields map onto a [`crate::registry::Project`]
/// the same way the add flow builds one; anything left `None` is filled by the
/// folder's own framework detection.
pub struct Recipe {
    /// Stable id used by `portbay_setup_from_recipe` (e.g. `laravel`).
    pub id: &'static str,
    /// Human-readable name.
    pub title: &'static str,
    /// One line on what the recipe sets up and when to pick it.
    pub description: &'static str,
    /// The project type this recipe registers.
    pub project_type: ProjectType,
    /// Default language version, when the stack pins one (PHP recipes).
    pub php_version: Option<&'static str>,
    /// Document root relative to the project, when the stack serves from a
    /// subdirectory (e.g. `public`).
    pub document_root: Option<&'static str>,
    /// Whether local HTTPS is on by default.
    pub https: bool,
    /// A recommended database engine (`engine:version`) when the stack expects
    /// one. Provisioning is a follow-on; for now this surfaces as guidance.
    pub needs_database: Option<&'static str>,
    /// Whether the stack benefits from a local mail catcher (Mailpit).
    pub needs_mail: bool,
}

/// The recipe catalog. Phase 1 covers stacks that compose fully from registry
/// state alone (framework + PHP version + document root + HTTPS). Stacks that
/// also expect a database or mail catcher are included where they still run
/// usefully without one; the rest arrive alongside bundled-service support.
const RECIPES: &[Recipe] = &[
    Recipe {
        id: "next",
        title: "Next.js",
        description: "Next.js app (React). Node dev server with HTTPS and a local hostname.",
        project_type: ProjectType::Next,
        php_version: None,
        document_root: None,
        https: true,
        needs_database: None,
        needs_mail: false,
    },
    Recipe {
        id: "vite",
        title: "Vite",
        description: "Vite-powered front end (React/Vue/Svelte/vanilla). Node dev server, HTTPS.",
        project_type: ProjectType::Vite,
        php_version: None,
        document_root: None,
        https: true,
        needs_database: None,
        needs_mail: false,
    },
    Recipe {
        id: "astro",
        title: "Astro",
        description: "Astro site. Node dev server with HTTPS and a local hostname.",
        project_type: ProjectType::Node,
        php_version: None,
        document_root: None,
        https: true,
        needs_database: None,
        needs_mail: false,
    },
    Recipe {
        id: "node",
        title: "Node",
        description: "Generic Node service. Runs the project's dev script with HTTPS.",
        project_type: ProjectType::Node,
        php_version: None,
        document_root: None,
        https: true,
        needs_database: None,
        needs_mail: false,
    },
    Recipe {
        id: "static",
        title: "Static site",
        description: "Plain HTML/CSS/JS served directly over HTTPS — no dev server.",
        project_type: ProjectType::Static,
        php_version: None,
        document_root: None,
        https: true,
        needs_database: None,
        needs_mail: false,
    },
    Recipe {
        id: "php",
        title: "PHP",
        description: "A plain PHP project served by Caddy + PHP-FPM over HTTPS.",
        project_type: ProjectType::Php,
        php_version: Some("8.3"),
        document_root: None,
        https: true,
        needs_database: None,
        needs_mail: false,
    },
    Recipe {
        id: "laravel",
        title: "Laravel",
        description: "Laravel app served from public/ by Caddy + PHP-FPM, HTTPS. \
                      Recommends a MySQL database and Mailpit.",
        project_type: ProjectType::Php,
        php_version: Some("8.3"),
        document_root: Some("public"),
        https: true,
        needs_database: Some("mysql:8.0"),
        needs_mail: true,
    },
    Recipe {
        id: "symfony",
        title: "Symfony",
        description: "Symfony app served from public/ by Caddy + PHP-FPM, HTTPS. \
                      Recommends a MySQL database and Mailpit.",
        project_type: ProjectType::Php,
        php_version: Some("8.3"),
        document_root: Some("public"),
        https: true,
        needs_database: Some("mysql:8.0"),
        needs_mail: true,
    },
    Recipe {
        id: "statamic",
        title: "Statamic",
        description: "Statamic (flat-file CMS) served from public/ by Caddy + PHP-FPM, HTTPS. \
                      No database required.",
        project_type: ProjectType::Php,
        php_version: Some("8.3"),
        document_root: Some("public"),
        https: true,
        needs_database: None,
        needs_mail: false,
    },
];

/// All recipes in the catalog.
pub fn all() -> &'static [Recipe] {
    RECIPES
}

/// Look up a recipe by id (case-insensitive).
pub fn find(id: &str) -> Option<&'static Recipe> {
    let id = id.trim().to_ascii_lowercase();
    RECIPES.iter().find(|r| r.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_non_empty_and_ids_unique() {
        let all = all();
        assert!(!all.is_empty());
        let mut ids: Vec<&str> = all.iter().map(|r| r.id).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "recipe ids must be unique");
    }

    #[test]
    fn find_is_case_insensitive() {
        assert!(find("Laravel").is_some());
        assert!(find("laravel").is_some());
        assert!(find("nope").is_none());
    }
}
