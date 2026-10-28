# Changelog

## [Unreleased]

### Added

- Subtasks with their own ID, name, status and properties, synchronized with GitHub sub-issues, Jira sub-tasks and Redmine parent issues
- Custom fields for Jira and Redmine, discovered automatically and pinned with `git task config fields list` and `save`
- Several Jira projects in one repository via `task.jira.projects`, with `--project` on pull, push and create
- Self-hosted Jira instances and Atlassian Document Format for Jira descriptions and comments
- Redmine projects via `task.redmine.project.id`, with issue moves between projects through the `project_id` property
- `git task pull --open` and `--status` to pull a subset of remote issues
- `--create` on `git task status` to add an unknown status on the fly
- `--force` when renaming a task onto an existing ID or pointing `task.ref` at an existing ref
- Case-insensitive status names and shortcuts
- Integration tests for GitHub, GitLab and Redmine

### Changed

- License switched from MIT to GPL-3.0-or-later
- Push synchronizes comments by default
- Push and pull return a failure exit code when any task operation fails
- Pull fetches open and closed Redmine issues by default
- Fetch limit applies across all pulled projects
- Redmine API key option renamed to `task.redmine.api.key`

### Fixed

- Markdown round trip through Jira now follows CommonMark for emphasis, code spans, links, lists and tables
- Connectors return errors instead of panicking on unexpected remote data
- Non-numeric and negative task IDs sort and parse correctly
- Date filters and `--until` handle datetimes and inclusive bounds correctly
- Status renames, deletions and shortcut lookups no longer accept conflicting or empty names
- Duplicate IDs, names and reserved property names are rejected on set, import and config commands
- Redmine journals without notes no longer become blank comments

## [0.2.4] - 2026-03-07

### Added

- `git task config connectors list` to show the configured connectors
- `--filter` option on `git task list`

### Fixed

- Redmine connector listing and issue handling

## [0.2.3] - 2025-12-09

### Added

- Redmine connector with listing, creation, update, deletion and comments
- `task.default.connector` and `task.list.show.headers` config options
- Fetch limit for Redmine

### Fixed

- Redmine API key parameter and task creation
- Error messages on failed remote operations

## [0.2.2] - 2025-04-23

### Added

- Jira connector with listing, creation, update, labels, comments and status transitions
- `git task comment set` command
- Optional `IN_PROGRESS` status for pulls
- Connector selection when several connectors match one remote
- Status shortcuts accepted when getting or setting status properties

### Changed

- Config options are collected from connectors instead of being hardcoded
- Rust edition updated

### Fixed

- Remote comment update for Jira
- Hardcoded Done status for Jira issues
- Error handling for `task get` and `task list`
- Color and ID range parsing

## [0.2.1] - 2024-12-25

### Added

- Labels for tasks, synchronized with GitHub and GitLab on push and pull
- Optional creation of missing project labels on push
- Synthetic `labels` property for listing

### Changed

- Reserved property names such as `id` and `labels` can no longer be added as custom properties
- Label colors formatted according to the GitHub and GitLab API specifications

## [0.2.0] - 2024-11-28

### Added

- `git task replace` command
- Conditional formatting of listed values based on expressions
- Integer property values in conditional formatting
- `git task set` operating on several tasks at once

### Changed

- Task IDs parameter format for commands accepting several tasks

## [0.1.2] - 2024-10-14

### Added

- Custom GitLab domains
- Deletion of GitLab issues and notes

## [0.1.1] - 2024-10-02

### Added

- GitLab connector with import, creation and update of issues and comments
- `git task unset` command
- `set` and `unset` operating on ID ranges
- Deletion of tasks by status
- Styles for statuses, properties and enum values

### Fixed

- Parsing of GitHub remote URLs
- `task.ref` configuration

## [0.1.0] - 2024-09-13

### Added

- Local task tracking stored in a git ref, with create, list, show, set, edit, delete, export and import
- GitHub connector with pull and push of issues and comments
- Configurable statuses with shortcuts, colors and styles
- Custom properties with enum values and conditional formatting
- Comments with editor support
- Filtering by status, author and date, sorting and listing limits
- Task ID ranges in commands
- Configurable task ref with data migration
- Git color settings and `NO_COLOR` support

[Unreleased]: https://github.com/jhspetersson/git-task/compare/0.2.4...HEAD
[0.2.4]: https://github.com/jhspetersson/git-task/compare/0.2.3...0.2.4
[0.2.3]: https://github.com/jhspetersson/git-task/compare/0.2.2...0.2.3
[0.2.2]: https://github.com/jhspetersson/git-task/compare/0.2.1...0.2.2
[0.2.1]: https://github.com/jhspetersson/git-task/compare/0.2.0...0.2.1
[0.2.0]: https://github.com/jhspetersson/git-task/compare/0.1.2...0.2.0
[0.1.2]: https://github.com/jhspetersson/git-task/compare/0.1.1...0.1.2
[0.1.1]: https://github.com/jhspetersson/git-task/compare/0.1.0...0.1.1
[0.1.0]: https://github.com/jhspetersson/git-task/releases/tag/0.1.0
