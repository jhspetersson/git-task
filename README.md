# git-task

Local-first task manager/bug tracker within your git repository which can import issues from GitHub, Gitlab or Jira Cloud.

[![Crates.io](https://img.shields.io/crates/v/git-task.svg)](https://crates.io/crates/fselect)
[![build](https://github.com/jhspetersson/git-task/actions/workflows/rust.yml/badge.svg)](https://github.com/jhspetersson/git-task/actions/workflows/rust.yml)

![](https://github.com/jhspetersson/git-task/blob/master/resources/gt2.gif)


[Installation](#installation)  
[Concepts](#concepts)  
[Commands](#commands)  
[GitHub support](#github-support)
[Gitlab support](#gitlab-support)  
[JIRA support](#jira-support)


## Installation

Build a binary, then add it as a git command:

    cargo install git-task
    git config --global alias.task '!~/.cargo/bin/git-task'

Now you can switch to some git repo and run it as:

    git task create "My first task"

Or import issues from GitHub (`GITHUB_TOKEN` or `GITHUB_API_TOKEN` env variable might be needed to be set up if you have a private repository)
or Gitlab (`GITLAB_TOKEN` or `GITLAB_API_TOKEN` is needed then):

    git task pull

## Concepts

`git-task` maintains a collection of tasks, which are essentially an integer ID and a set of properties.

Some properties are special: `name`, `description`, `author`, `created` and `status`. You can add custom properties for every task.
It's possible to define conditional color highlighting depending on the value of the property. 

Tasks can have comments that are also addressed by their ID.

Status can be anything, but it is expected to be one of the several defined statuses. 
By default, there are three: `OPEN` for new tasks, `IN_PROGRESS` for the tasks that are in development, `CLOSED` for complete ones.
You can freely edit this list.

However, for the sake of sync with GitHub or Gitlab there are two config options to map remote `OPEN` and `CLOSED` statuses with local ones.

Tasks can also have labels that are optionally synchronized with GitHub or Gitlab.

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

### get

Prints task property.

    git task get 1 description

### set

Sets task property:

    git task set 1 description "I figured it out all wrong. Fizz Buzz has to be rewritten in Rust!"
    git task set 1..10 priority HIGH

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

    git task pull -s o
    git task pull --status OPEN

### push

Push the status of the selected tasks to the remote source.

    git task push 2,3,4,5,10,12
    git task push 2..5,10,12

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
    git task config get task.list.sort
    git task config get task.status.open
    git task config get task.status.closed
    git task config get task.ref

Customize sorting:

    git task config set task.list.sort "created desc"

Customize columns:

    git task config set task.list.columns id,author,status,name

By default `git-task` saves everything under a custom ref. You can change that to a regular branch like this:

    git task config set task.ref refs/heads/tasks

Remove the old ref after setting a new one:

    git task config set task.ref refs/heads/tasks --move

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

    git task cfg prop cond add id "status == \"CLOSED\"" DarkGray strikethrough
    git task cfg prop cond add name "status == \"CLOSED\"" DarkGray strikethrough

Conditional formatting has a precedence over enum values, which supersede the default color and style of the defined property. 

Clear conditional formatting:

    git task cfg prop cond clear id
    git task cfg prop cond clear name

You can also export, manually edit and import back task properties configuration.

    git task config props export
    git task config props import
    git task config props reset

### help

Show available commands or their arguments:

    git task help
    git task help create

## GitHub support

For private repositories you have to set up `GITHUB_TOKEN` or `GITHUB_API_TOKEN` environment variable for GitHub.

## Gitlab support

For any operation you will need to set up `GITLAB_TOKEN` or `GITLAB_API_TOKEN` environment variable.

For custom domains please set up `GITLAB_URL` variable. Alternatively, you can set the custom domain in git config:

    git task config set task.gitlab.url gitlab.kitware.com

## JIRA support

Set up a Jira Cloud URL:

    git task config set task.jira.url https://someuser.atlassian.net/jira/software/projects/GTPM

Set up a Jira Cloud user:

    git task config set task.jira.user someuser@example.com

For any operation you will need to set up `JIRA_TOKEN` or `JIRA_API_TOKEN` environment variable.

We also recommend setting up statuses as they are organized in Jira.

## Redmine support

Set up a Redmine URL:

    git task config set task.redmine.url https://redmine.example.com

Alternatively, you can set the `REDMINE_URL` environment variable.

Set up a Redmine API key:

    git task config set task.redmine.api_key your_api_key_here

Alternatively, you can set the `REDMINE_API_KEY` or `REDMINE_TOKEN` environment variable.

## License

MIT

---

Supported by [JetBrains IDEA](https://jb.gg/OpenSourceSupport) open source license
