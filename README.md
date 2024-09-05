# git-task

Local-first task manager/bug tracker within your git repository which can import issues from GitHub.

Current state: PoC, but it works for me.

## Installation

Build a binary, then place it somewhere and add it as a git command:

    git clone https://github.com/jhspetersson/git-task
    cd git-task
    cargo build --release
    sudo cp target/release/git-task /usr/local/bin/git-task
    git config --global alias.task '!/usr/local/bin/git-task'

Now you can switch to some git repo and run it as:

    git task create "My first task"

Or import issues from GitHub (`GITHUB_TOKEN` or `GITHUB_API_TOKEN` env variable might be needed to be set up if you have a private repository):

    git task pull

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

Filter by keyword:

    git task list -k linux

Filter by date:

    git task list --from 2024-01-01
    git task list --until 2023-12-31

Filter by author:

    git task list --author jhspetersson

Show specific columns:

    git task list --columns id,status,name

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

### get

Prints task property.

    git task get 1 description

### set

Sets task property.

    git task set 1 description "I figured it out all wrong. Fizz Buzz has to be rewritten in Rust!"

### edit

Edit task property in the default git editor.

    git task edit 1 description

### comment

Add or remove comments:

    git task comment add 1 "This is a comment to my first task"
    git task comment del 1 1

### import

Import all or selected tasks from JSON file.

    git task import <my_tasks.json
    git task import 2 3 5 <my_tasks.json

### export

Export all or selected tasks, only JSON output format is currently supported.

    git task export
    git task export --pretty 2 3 5 >my_tasks.json

### pull

Grab issues from remote source (currently, only GitHub is supported).
For private repositories you have to set up `GITHUB_TOKEN` or `GITHUB_API_TOKEN` environment variable.

    git task pull
    git task pull --no-comments
    git task pull 2 3 5
    git task pull --limit 50

Pull only open issues:

    git task pull -s o
    git task pull --status OPEN

### push

Push status of the selected tasks to the remote source.
For GitHub you have to set up `GITHUB_TOKEN` or `GITHUB_API_TOKEN` environment variable.

    git task push 2 3 5

### stats

Show total task count, count by status and top 10 authors.

    git task stats

### delete

Deletes one or more tasks by their IDs.

    git task delete 1
    git task delete 2 3 5

Also delete a corresponding GitHub issue:

    git task delete 120 --push

### clear

Deletes all tasks.

    git task clear

### config

Maintain configuration parameters.

    git task config list
    git task config get task.ref

By default `git-task` saves everything under a custom ref. You can change that to a regular branch like this:

    git task config set task.ref refs/heads/tasks

Remove old ref after setting a new one:

    git task config set task.ref refs/heads/tasks --move

Configure task statuses:

    git task config status list
    git task config status set CLOSED color Magenta
    git task config status set CLOSED name FINISHED
    git task config status set FINISHED shortcut f

You can export status config, edit it manually and import it back:

    git task config status export --pretty >statuses.json
    git task config status import <statuses.json

### help

Show available commands or their arguments.

    git task help
    git task help create

## License

MIT

---

Supported by [JetBrains IDEA](https://jb.gg/OpenSourceSupport) open source license
