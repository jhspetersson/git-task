mod github;
mod operations;
mod status;
mod util;

extern crate gittask;

use clap::{Parser, Subcommand};

use crate::operations::{task_clear, task_comment_add, task_comment_delete, task_config_get, task_config_list, task_config_set, task_config_status_export, task_config_status_get, task_config_status_import, task_config_status_list, task_config_status_set, task_create, task_delete, task_edit, task_export, task_get, task_import, task_list, task_pull, task_push, task_set, task_show, task_stats, task_status};

#[derive(Parser)]
#[command(arg_required_else_help(true))]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List all tasks
    List {
        /// Filter by status (o - OPEN, i - IN_PROGRESS, c - CLOSED)
        #[arg(short, long)]
        status: Option<String>,
        /// Filter by keyword
        #[arg(short, long)]
        keyword: Option<String>,
        /// Newer than date, YYYY-MM-DD, inclusive
        #[arg(short, long)]
        from: Option<String>,
        /// Older than date, YYYY-MM-DD, inclusive
        #[arg(short, long)]
        until: Option<String>,
        /// Filter by author
        #[arg(long)]
        author: Option<String>,
        /// Comma-separated list of columns
        #[arg(short, long, value_delimiter = ',')]
        columns: Option<Vec<String>>,
        /// Soring by one or more task properties, e.g. --sort "author, created desc"
        #[arg(long, value_delimiter = ',')]
        sort: Option<Vec<String>>,
        /// Limit displayed task count
        #[arg(short, long)]
        limit: Option<usize>,
        /// Disable colors
        #[arg(long)]
        no_color: bool,
    },
    /// Show a task with all properties
    Show {
        /// task ID
        id: String,
        /// Disable colors
        #[arg(long)]
        no_color: bool,
    },
    /// Create a new task
    Create {
        /// task name
        name: String,
        /// task description
        description: Option<String>,
        /// Skip editing description in the editor
        #[arg(short, long, conflicts_with = "description")]
        no_desc: bool,
        /// Also push task to the remote source (e.g., GitHub)
        #[arg(short, long)]
        push: bool,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
    },
    /// Update task status
    Status {
        /// task ID
        id: String,
        /// status (o - OPEN, i - IN_PROGRESS, c - CLOSED)
        status: String,
    },
    /// Get a property
    Get {
        /// task ID
        id: String,
        /// property name
        prop_name: String,
    },
    /// Set a property
    Set {
        /// task ID
        id: String,
        /// property name
        prop_name: String,
        /// property value
        value: String,
    },
    /// Edit a property
    Edit {
        /// task ID
        id: String,
        /// property name
        prop_name: String,
    },
    /// Add or delete comments
    Comment {
        #[command(subcommand)]
        subcommand: CommentCommand,
    },
    /// Import tasks from a source
    Import {
        /// space separated task IDs
        ids: Option<Vec<String>>,
        /// Input format (only JSON is currently supported)
        #[arg(short, long)]
        format: Option<String>,
    },
    /// Export tasks
    Export {
        /// space separated task IDs
        ids: Option<Vec<String>>,
        /// Output format (only JSON is currently supported)
        #[arg(short, long)]
        format: Option<String>,
        /// Prettify output
        #[arg(short, long)]
        pretty: bool,
    },
    /// Import tasks from a remote source (e.g., GitHub)
    Pull {
        /// space separated task IDs
        ids: Option<Vec<String>>,
        /// Limit the count of imported tasks
        #[arg(short, long, conflicts_with = "ids")]
        limit: Option<usize>,
        /// Import only issues with this status
        #[arg(short, long, conflicts_with = "ids")]
        status: Option<String>,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
        /// Don't import task comments
        #[arg(short, long)]
        no_comments: bool,
    },
    /// Push task status to the remote source (e.g., GitHub)
    Push {
        /// space separated task IDs
        ids: Vec<String>,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
        /// Don't create task comments
        #[arg(short, long)]
        no_comments: bool,
        /// Disable colors
        #[arg(long)]
        no_color: bool,
    },
    /// Show total task count and count by status
    Stats {
        /// Disable colors
        #[arg(long)]
        no_color: bool,
    },
    /// Delete one or several tasks at once
    #[clap(visible_aliases(["del", "remove", "rem"]))]
    Delete {
        /// space separated task IDs
        ids: Vec<String>,
        /// Also delete task from the remote source (e.g., GitHub)
        #[arg(short, long)]
        push: bool,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
    },
    /// Delete all tasks
    Clear,
    /// Set configuration parameters
    Config {
        #[command(subcommand)]
        subcommand: ConfigCommand,
    },
}

#[derive(Subcommand)]
enum CommentCommand {
    /// Add a comment
    Add {
        /// task ID
        task_id: String,
        /// comment text
        text: String,
        /// Also push comment to the remote source (e.g., GitHub)
        #[arg(short, long)]
        push: bool,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
    },
    /// Delete a comment
    #[clap(visible_aliases(["del", "remove", "rem"]))]
    Delete {
        /// task ID
        task_id: String,
        /// comment ID
        comment_id: String,
        /// Also delete comment from the remote source (e.g., GitHub)
        #[arg(short, long)]
        push: bool,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Get configuration parameter
    Get {
        /// parameter name
        param: String,
    },
    /// Set configuration parameter
    Set {
        /// parameter name
        param: String,
        /// parameter value
        value: String,
        /// Remove old tasks ref after update
        #[arg(long = "move")]
        move_ref: bool,
    },
    /// List configuration parameters
    List,
    /// Configure task statuses
    Status {
        #[command(subcommand)]
        subcommand: StatusCommand,
    },
}

#[derive(Subcommand)]
enum StatusCommand {
    /// Get task status
    Get {
        /// status name
        name: String,
        /// status parameter
        param: String,
    },
    /// Set task status configuration
    Set {
        /// status name
        name: String,
        /// status parameter
        param: String,
        /// parameter value
        value: String,
    },
    /// List task statuses
    List,
    /// Import task statuses from JSON
    Import,
    /// Export task statuses
    Export {
        /// Prettify output
        #[arg(short, long)]
        pretty: bool,
    },
}

fn main() {
    let _ = enable_ansi_support::enable_ansi_support();
    let args = Args::parse();
    match args.command {
        Some(Command::List { status, keyword, from, until, author, columns, sort, limit, no_color }) => task_list(status, keyword, from, until, author, columns, sort, limit, no_color),
        Some(Command::Show { id, no_color }) => task_show(id, no_color),
        Some(Command::Create { name, description, no_desc, push, remote }) => task_create(name, description, no_desc, push, remote),
        Some(Command::Status { id, status }) => task_status(id, status),
        Some(Command::Get { id, prop_name }) => task_get(id, prop_name),
        Some(Command::Set { id, prop_name, value }) => task_set(id, prop_name, value),
        Some(Command::Edit { id, prop_name }) => task_edit(id, prop_name),
        Some(Command::Comment { subcommand }) => task_comment(subcommand),
        Some(Command::Import { ids, format }) => task_import(ids, format),
        Some(Command::Export { ids, format, pretty }) => task_export(ids, format, pretty),
        Some(Command::Pull { ids, limit, status, remote, no_comments }) => task_pull(ids, limit, status, remote, no_comments),
        Some(Command::Push { ids, remote, no_comments, no_color }) => task_push(ids, remote, no_comments, no_color),
        Some(Command::Stats { no_color }) => task_stats(no_color),
        Some(Command::Delete { ids, push, remote }) => task_delete(ids, push, remote),
        Some(Command::Clear) => task_clear(),
        Some(Command::Config { subcommand }) => task_config(subcommand),
        None => { }
    }
}

fn task_comment(subcommand: CommentCommand) {
    match subcommand {
        CommentCommand::Add { task_id, text, push, remote } => task_comment_add(task_id, text, push, remote),
        CommentCommand::Delete { task_id, comment_id, push, remote } => task_comment_delete(task_id, comment_id, push, remote),
    }
}

fn task_config(subcommand: ConfigCommand) {
    match subcommand {
        ConfigCommand::Get { param } => task_config_get(param),
        ConfigCommand::Set { param, value, move_ref } => task_config_set(param, value, move_ref),
        ConfigCommand::List => task_config_list(),
        ConfigCommand::Status { subcommand } => task_config_status(subcommand),
    }
}

fn task_config_status(subcommand: StatusCommand) {
    match subcommand {
        StatusCommand::Get { name, param } => task_config_status_get(name, param),
        StatusCommand::Set { name, param, value } => task_config_status_set(name, param, value),
        StatusCommand::List => task_config_status_list(),
        StatusCommand::Import => task_config_status_import(),
        StatusCommand::Export { pretty } => task_config_status_export(pretty),
    }
}