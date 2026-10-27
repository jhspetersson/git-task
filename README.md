# git-task

Local-first task manager/bug tracker within your git repository which can import issues from GitHub, GitLab, Jira Cloud and Redmine.

[![Crates.io](https://img.shields.io/crates/v/git-task.svg)](https://crates.io/crates/fselect)
[![build](https://github.com/jhspetersson/git-task/actions/workflows/rust.yml/badge.svg)](https://github.com/jhspetersson/git-task/actions/workflows/rust.yml)

![](https://github.com/jhspetersson/git-task/blob/master/resources/gt2.gif)


[Installation](#installation)  
[Concepts](#concepts)  
[Commands](#commands)  
[GitHub support](#github-support)  
[GitLab support](#gitlab-support)  
[JIRA support](#jira-support)  
[Redmine support](#redmine-support)


## Installation

Build a binary, then add it as a git command:

    cargo install git-task
    git config --global alias.task '!~/.cargo/bin/git-task'

Now you can switch to some git repo and run it as:

    git task create "My first task"

Or import issues from GitHub (`GITHUB_TOKEN` or `GITHUB_API_TOKEN` env variable might be needed to be set up if you have a private repository)
or GitLab (`GITLAB_TOKEN` or `GITLAB_API_TOKEN` is needed then):

    git task pull

## Concepts

`git-task` maintains a collection of tasks, which are essentially an integer ID and a set of properties.

Some properties are special: `name`, `description`, `author`, `created` and `status`. You can add custom properties for every task.
It's possible to define conditional color highlighting depending on the value of the property. 

Tasks can have comments that are also addressed by their ID.

Status can be anything, but it is expected to be one of the several defined statuses. 
By default, there are three: `OPEN` for new tasks, `IN_PROGRESS` for the tasks that are in development, `CLOSED` for complete ones.
You can freely edit this list.

However, for the sake of sync with GitHub or GitLab there are two config options to map remote `OPEN` and `CLOSED` statuses with local ones.

Tasks can also have labels that are optionally synchronized with GitHub or GitLab.

Tasks can be broken down into **subtasks** — lightweight items with an ID, name, status, and optional custom properties, stored inside the parent task.
Subtasks are included in JSON import/export and can be synchronized with remotes that support them natively: GitHub sub-issues, Jira sub-tasks, and Redmine parent/child issues.

## Commands

### list

Lists all tasks.

    git task list

Show only open tasks:

    git task list -s o
    git task list --status o

Show only closed tasks:

    git task list -s c

Show only tasks with a custom status:

    git task list --status DEPLOYED

Show tasks that are new or in progress:

    git task list -s OPEN,IN_PROGRESS
    git task list -s o,i

Filter by keyword:

    git task list -k linux

Filter by date:

    git task list --from 2024-01-01
    git task list --until 2023-12-31

Filter by author:

    git task list --author jhspetersson

Filter by expression:

    git task list --filter 'status == "OPEN" && author == "jhspetersson"'
    git task list --filter 'priority > 1 || status == "IN_PROGRESS"'
    git task list --filter "id > 100"

Expression language used: [evalexpr](https://github.com/ISibboI/evalexpr).

Show specific columns:

    git task list --columns id,status,name

Show column names:

    git task list --headers

Sorting by one or more task properties:

    git task list --sort author
    git task list --sort "status, created desc"

Limit displayed task count:

    git task list -l 10
    git task list --limit 5

### show

Shows one task with all the properties (like id, name, status, description and a bunch of custom ones, actually, you can add whatever you like).

    git task show 1

### create

Creates a new task.

    git task create "Fix my Fizz Buzz implementation"
    git task create "Task title" "Task description"
    git task create "This task goes without description" --no-desc
    git task create "Create a task and push it to GitHub" --push

### status

Updates task status.

    git task status 1 IN_PROGRESS
    git task status 1 i
    git task status 2..5,10,12 c

The status must be one of the configured statuses (see `git task config status list`), given by name or shortcut. Unknown statuses are rejected. Pass `--create` to add a missing status on the fly; it gets a free shortcut and the `Default` color, which you can adjust later with `git task config status set`:

    git task status 1 BLOCKED --create

### get

Prints task property.

    git task get 1 description

### set

Sets task property:

    git task set 1 description "I figured it out all wrong. Fizz Buzz has to be rewritten in Rust!"
    git task set 1..10 priority HIGH

Changing the `id` property renames the task. It fails if a task with the new ID already exists, unless `--force` is given, which replaces that task:

    git task set 1 id 42
    git task set 1 id 42 --force

### replace

Search and replace within property values:

    git task replace 1..10 description "Acme" "ACME Corp."
    git task replace 1..10 description "Acme" "ACME Corp." --push

### unset

Delete a property:

    git task unset 1 foo
    git task unset 1..10 foo

### edit

Edit task property in the default git editor.

    git task edit 1 description

Editing the `id` property follows the same rules as `set`, including `--force`.

For Windows, we recommend anything, but `notepad`. `Notepad++` (a separate installation) is just fine.
You can set it up this way:

    git config --global core.editor "C:\\Program Files\\Notepad++\\notepad++.exe"

### label

Add and remove labels from tasks. Labels can be synchronized with the remote sources.

    git task label add 10 important ff6633 --desc 'Beware of this task!' --push
    git task lbl del 10 important

### comment

Add, set, edit or remove comments:

    git task comment add 1 "This is a comment to my first task"
    git task comment set 1 1 "Old comment has been replaced with this one!"    
    git task comment edit 1 1
    git task comment del 1 1

You can sync comments with the remote source:

    git task comment edit 159 2334900009 --push

### subtask

Break a task into smaller pieces. Subtasks live inside the parent task and carry an ID, name, status, and any custom properties you want to set.

Add a subtask:

    git task subtask add 1 "Write unit tests"
    git task sub add 1 "Deploy to staging" --status IN_PROGRESS

List subtasks of a task:

    git task subtask list 1

Update a subtask's name, status or any custom property:

    git task subtask set 1 2 status CLOSED
    git task sub set 1 2 name "Deploy to production"
    git task sub set 1 2 priority HIGH

Edit a subtask in the default editor:

    git task subtask edit 1 2
    git task subtask edit 1 2 status

Delete a subtask:

    git task subtask del 1 2

Subtasks are stored inside the parent task, so `git task export` and `git task import` carry them along automatically.

When pushing or pulling, subtasks are synchronized with remotes that expose a native sub-issue / sub-task concept. GitHub sub-issues, Jira sub-tasks, and Redmine parent/child issues are supported out of the box:

    git task subtask add 1 "Fix bug" --push
    git task push 1
    git task pull 1

To skip subtask sync on push or pull:

    git task push 1 --no-subtasks
    git task pull 1 --no-subtasks

### import

Import all or selected tasks from a JSON file.

    git task import <my_tasks.json
    git task import 2,3,4,5,10,12 <my_tasks.json
    git task import 2..5,10,12 <my_tasks.json

### export

Export all or selected tasks, only JSON output format is currently supported.

    git task export
    git task export --pretty 2,3,4,5,10,12 >my_tasks.json
    git task export --pretty 2..5,10,12 >my_tasks.json
    git task export --status o,i
    git task export --limit 50

### pull

Grab issues from a remote source.

    git task pull
    git task pull --no-comments
    git task pull 2,3,4,5,10,12
    git task pull 2..5,10,12
    git task pull --limit 50

Pull only open issues:

    git task pull --open

Pull only issues with a particular status:

    git task pull -s o
    git task pull --status OPEN
    git task pull --status "In Progress"

When the connector manages several projects (see [JIRA support](#jira-support)), pull one of them or all of them:

    git task pull --project OPS
    git task pull

### push

Push the status of the selected tasks to the remote source.

    git task push 2,3,4,5,10,12
    git task push 2..5,10,12
    git task push GTPM-42,OPS-7

### stats

Show the total task count, count by status and top 10 authors.

    git task stats

### delete

Deletes one or more tasks by their IDs or status.

    git task delete 1
    git task delete 2,3,4,5,10,12
    git task delete 2..5,10,12
    git task delete -s CLOSED
    git task delete -s c

Also, delete a corresponding remote issue:

    git task delete 120 --push

### clear

Deletes all tasks.

    git task clear

### config

Maintain configuration parameters.

    git task config list
    git task config get task.list.columns
    git task config get task.list.show.headers
    git task config get task.list.sort
    git task config get task.status.open
    git task config get task.status.closed
    git task config get task.default.connector
    git task config get task.ref

Customize sorting:

    git task config set task.list.sort "created desc"

Customize columns:

    git task config set task.list.columns id,author,status,name

Show column headers every time:

    git task config set task.list.show.headers true

Set default connector type when it can't be detected automatically:

    git task config set task.default.connector redmine

By default `git-task` saves everything under a custom ref. You can change that to a regular branch like this:

    git task config set task.ref refs/heads/tasks

Remove the old ref after setting a new one:

    git task config set task.ref refs/heads/tasks --move

If the new ref already exists and points elsewhere, the command refuses to touch it. Overwrite it anyway:

    git task config set task.ref refs/heads/tasks --force

Configure task statuses:

    git task config status list
    git task config status set CLOSED color Magenta
    git task config status set c color Magenta
    git task config status set CLOSED name FINISHED
    git task config status set FINISHED shortcut f
    git task config status set f style bold,italic
    git task config set task.status.closed FINISHED

Colors available:

    Black, DarkGray, Red, LightRed, Green, LightGreen, Yellow, LightYellow, Blue, LightBlue, Purple, LightPurple, Magenta, LightMagenta, Cyan, LightCyan, White, LightGray

Or a one-byte value like:

    239

Styles available:

    bold, dimmed, italic, normal, strikethrough, underline

Add and delete statuses:

    git task config status add ARCHIVE a Magenta true
    git task config status delete ARCHIVE
    git task config status delete a

You can export status config, edit it manually and import it back:

    git task config status export --pretty >statuses.json
    git task config status import <statuses.json

If everything went wrong:

    git task config status reset

Configure known task properties (you can add any other if you wish to any task):

    git task config props add client_name string Cyan
    git task config props set client_name color Blue
    git task config props delete client_name

You can also set up their own colors for specific values of the properties (assuming you've already added `priority` property):

    git task config prop enum add priority HIGH Red
    git task config prop enum get priority HIGH color
    git task config prop enum set priority HIGH Magenta bold
    git task config prop enum list priority    
    git task config prop enum del priority HIGH

You can go even further and set up conditional formatting (color and style) to any property depending on a boolean expression.
Expression language used: [evalexpr](https://github.com/ISibboI/evalexpr).

Task properties are automatically exported to the evaluation context as string or integer values.

For example, we want task ID and names to be rendered with dark gray color and strikethrough style if the status is `CLOSED` 
(like they do it in JetBrains products, e.g., YouTrack):

    git task cfg prop cond add id 'status == "CLOSED"' DarkGray strikethrough
    git task cfg prop cond add name 'status == "CLOSED"' DarkGray strikethrough

Conditional formatting has a precedence over enum values, which supersede the default color and style of the defined property. 

Clear conditional formatting:

    git task cfg prop cond clear id
    git task cfg prop cond clear name

You can also export, manually edit and import back task properties configuration.

    git task config props export
    git task config props import
    git task config props reset

#### connectors

List configured connectors:

    git task connectors list

### help

Show available commands or their arguments:

    git task help
    git task help create

## GitHub support

For private repositories you have to set up `GITHUB_TOKEN` or `GITHUB_API_TOKEN` environment variable for GitHub.

Subtasks are synchronized using GitHub's sub-issues REST API. A local subtask that is not on GitHub yet is created as a fresh issue and then linked as a sub-issue of the parent. The local subtask ID is replaced with the new issue number. Sub-issues only know open and closed, so on pull they get your starting or final status, and on push a subtask is closed when its status is marked as done.

## GitLab support

For any operation you will need to set up `GITLAB_TOKEN` or `GITLAB_API_TOKEN` environment variable.

For custom domains please set up `GITLAB_URL` variable. Alternatively, you can set the custom domain in git config:

    git task config set task.gitlab.url gitlab.kitware.com

## JIRA support

Set up a Jira Cloud URL:

    git task config set task.jira.url https://someuser.atlassian.net/jira/software/projects/GTPM

Self-hosted instances work too. The project key is taken from the URL, or can be set explicitly:

    git task config set task.jira.url https://jira.example.com
    git task config set task.jira.project GTPM

Set up a Jira Cloud user:

    git task config set task.jira.user someuser@example.com

For any operation you will need to set up `JIRA_TOKEN` or `JIRA_API_TOKEN` environment variable.

We also recommend setting up statuses as they are organized in Jira.

Subtasks are synchronized as Jira sub-tasks: a local subtask is created as an issue with `issuetype = "Sub-task"` and a `parent` pointing to the task's issue key.

### Several Jira projects in one repository

List every project you want to manage:

    git task config set task.jira.projects GTPM,OPS,DOCS

In this mode local task IDs are full Jira issue keys (`GTPM-42`), so tasks from different projects never collide. Commands that take task IDs derive the project from the key, and `--project` selects one explicitly:

    git task pull                          # every configured project
    git task pull --project OPS
    git task pull GTPM-42,OPS-7
    git task push GTPM-42,OPS-7
    git task create "New task" --push --project DOCS
    git task comment add OPS-7 "Looks good" --push

Without `--project`, new tasks go to the default project: `task.jira.project`, then the project from `task.jira.url`, then the first entry of `task.jira.projects`.

Tasks pulled before enabling this mode have numeric IDs that belong to the default project. Rename them, together with their numeric subtask IDs, in one go when switching:

    git task config set task.jira.projects GTPM,OPS,DOCS --rename-tasks

## Redmine support

Set up a Redmine URL:

    git task config set task.redmine.url https://redmine.example.com

Alternatively, you can set the `REDMINE_URL` environment variable.

Set up a Redmine API key:

    git task config set task.redmine.api.key your_api_key_here

Alternatively, you can set the `REDMINE_API_KEY` or `REDMINE_TOKEN` environment variable.

Set default connector to use:

    git task config set task.default.connector redmine

Subtasks are synchronized using Redmine's `parent_issue_id`: a local subtask is created as a regular issue whose parent is the task, and children are listed via the `parent_id` filter.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE) for the full text.

---

Supported by [JetBrains IDEA](https://jb.gg/OpenSourceSupport) open source license
