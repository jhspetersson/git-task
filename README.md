# git-task

Local-first task manager/bug tracker within your git repository which can import issues from GitHub.

Current state: PoC, but it works for me.

## Installation

Build a binary, then place it somewhere and add it as a git command:

    git clone https://github.com/jhspetersson/git-task
    cd git-task
    cargo build --release
    cp target/release/git-task /usr/local/bin/git-task
    git config --global alias.task "!/usr/local/bin/git-task"

Now you can switch to some git repo and run it as:

    git task create "My first task"

Or import issues from GitHub if you have a public project with issues accessible:

    git task import

## Commands

### list

Lists all tasks.

    git task list

Show only open tasks:

    git task list -s o
    git task list --status o

Filter by keyword:

    git task list -k linux

Filter by date:

    git task list --from 2024-01-01
    git task list --until 2023-12-31

Show specific columns:

    git task list --columns id,status,name

### show

Shows one task with all the properties (like id, name, status, description and a bunch of custom ones, actually, you can add whatever you like).

    git task show 1

### create

Creates a new task.

    git task create "Fix my Fizz Buzz implementation"

### status

Updates task status.

    git task status 1 IN_PROGRESS

### get

Prints task property.

    git task get 1 description

### set

Sets task property

    git task set 1 description "I figured it out all wrong. Fizz Buzz has to be rewritten in Rust!"

### import

Import tasks from external source. Currently only JSON input and GitHub are supported.

    git task import
    git task import <my_tasks.json

### export

Export all or selected tasks, only JSON output format is currently supported.

    git task export
    git task export --pretty 2 3 5 >my_tasks.json

### delete

Deletes one or more tasks by their IDs.

    git task delete 1
    git task delete 2 3 5

### clear

Deletes all tasks.

    git task clear

### help

Show available commands or their arguments.

    git task help
    git task help create

## License

MIT

---

Supported by [JetBrains IDEA](https://jb.gg/OpenSourceSupport) open source license
