mod connectors;
mod operations;
mod property;
mod status;
mod util;

extern crate gittask;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::operations::{task_clear, task_comment_add, task_comment_delete, task_comment_edit, task_create, task_delete, task_edit, task_export, task_get, task_import, task_list, task_pull, task_push, task_set, task_show, task_stats, task_status, task_unset};
use crate::operations::config::*;

#[derive(Parser)]
#[command(version, about = "Local-first task manager/bug tracker within your git repository which can sync issues from GitHub.", arg_required_else_help(true))]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List all tasks
    List {
        /// Filter by status (by default: o - OPEN, i - IN_PROGRESS, c - CLOSED)
        #[arg(short, long, value_delimiter = ',')]
        status: Option<Vec<String>>,
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
    #[clap(visible_aliases(["add", "new"]))]
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
        /// one or more task IDs (space separated or a range like 1..10)
        #[clap(required = true)]
        ids: Vec<String>,
        /// status (by default: o - OPEN, i - IN_PROGRESS, c - CLOSED)
        #[clap(required = true)]
        status: String,
        /// Also push task(s) to the remote source (e.g., GitHub)
        #[arg(short, long)]
        push: bool,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
        /// Disable colors
        #[arg(long)]
        no_color: bool,
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
        /// Also push task to the remote source (e.g., GitHub)
        #[arg(short, long)]
        push: bool,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
        /// Disable colors
        #[arg(long)]
        no_color: bool,
    },
    /// Delete a property
    Unset {
        /// one or more task IDs (space separated or a range like 1..10)
        ids: Vec<String>,
        /// property name
        prop_name: String,
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
        /// one or more task IDs (space separated or a range like 1..10)
        ids: Option<Vec<String>>,
        /// Input format (only JSON is currently supported)
        #[arg(short, long)]
        format: Option<String>,
    },
    /// Export tasks
    Export {
        /// one or more task IDs (space separated or a range like 1..10)
        ids: Option<Vec<String>>,
        /// Filter by status (by default: o - OPEN, i - IN_PROGRESS, c - CLOSED)
        #[arg(short, long, value_delimiter = ',')]
        status: Option<Vec<String>>,
        /// Limit exported task count
        #[arg(short, long)]
        limit: Option<usize>,
        /// Output format (only JSON is currently supported)
        #[arg(short, long)]
        format: Option<String>,
        /// Prettify output
        #[arg(short, long)]
        pretty: bool,
    },
    /// Pull tasks from a remote source (e.g., GitHub)
    Pull {
        /// one or more task IDs (space separated or a range like 1..10)
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
        /// one or more task IDs (space separated or a range like 1..10)
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
        /// one or more task IDs (space separated or a range like 1..10)
        #[clap(required = true)]
        ids: Option<Vec<String>>,
        /// Delete by status (by default: o - OPEN, i - IN_PROGRESS, c - CLOSED)
        #[arg(short, long, value_delimiter = ',', conflicts_with = "ids", required_unless_present = "ids")]
        status: Option<Vec<String>>,
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
    #[clap(visible_aliases(["cfg"]))]
    Config {
        #[command(subcommand)]
        subcommand: ConfigCommand,
    },
}

#[derive(Subcommand)]
enum CommentCommand {
    /// Add a comment
    #[clap(visible_aliases(["create", "new"]))]
    Add {
        /// task ID
        task_id: String,
        /// comment text
        text: Option<String>,
        /// Also push comment to the remote source (e.g., GitHub)
        #[arg(short, long)]
        push: bool,
        /// Use this remote if there are several of them
        #[arg(short, long)]
        remote: Option<String>,
    },
    /// Edit a comment
    Edit {
        /// task ID
        task_id: String,
        /// comment ID
        comment_id: String,
        /// Also update comment on the remote source (e.g., GitHub)
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
    /// Configure task properties
    #[clap(visible_aliases(["props", "prop"]))]
    Properties {
        #[command(subcommand)]
        subcommand: PropertiesCommand,
    },
}

#[derive(Subcommand)]
enum StatusCommand {
    /// Add a status
    #[clap(visible_aliases(["create", "new"]))]
    Add {
        /// status name
        name: String,
        /// status shortcut
        shortcut: String,
        /// status color
        color: String,
        /// is it a final status?
        is_done: Option<bool>,
    },
    /// Delete a status
    #[clap(visible_aliases(["del", "remove", "rem"]))]
    Delete {
        /// status name
        name: String,
        /// Delete a status even there are tasks that have it
        #[arg(short, long)]
        force: bool,
    },
    /// Get task status parameter
    Get {
        /// status name
        name: String,
        /// status parameter
        param: String,
    },
    /// Set task status parameter
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
    /// Reset status configuration to default
    Reset,
}

#[derive(Subcommand)]
enum PropertiesCommand {
    /// Add a property
    #[clap(visible_aliases(["create", "new"]))]
    Add {
        /// property name
        name: String,
        /// property value type (string, text, datetime or integer)
        value_type: String,
        /// property color
        color: String,
        /// property enum value and color pairs
        enum_values: Option<Vec<String>>,
    },
    /// Delete a property
    #[clap(visible_aliases(["del", "remove", "rem"]))]
    Delete {
        /// property name
        name: String,
        /// Delete a property even there are tasks that have it
        #[arg(short, long)]
        force: bool,
    },
    /// Get task property parameter
    Get {
        /// property name
        name: String,
        /// property parameter (name, color or value_type)
        param: String,
    },
    /// Set task property parameter
    Set {
        /// property name
        name: String,
        /// property parameter (name, color or value_type)
        param: String,
        /// property value
        value: String,
    },
    /// Configure enum values of the property
    #[clap(visible_aliases(["enums"]))]
    Enum {
        #[command(subcommand)]
        subcommand: PropertiesEnumCommand,
    },
    /// List task properties
    List,
    /// Import task properties from JSON
    Import,
    /// Export task properties
    Export {
        /// Prettify output
        #[arg(short, long)]
        pretty: bool,
    },
    /// Reset properties configuration to default
    Reset,
}

#[derive(Subcommand)]
enum PropertiesEnumCommand {
    /// List enum values of a property
    List {
        /// property name
        name: String,
    },
    /// Add a property enum value
    #[clap(visible_aliases(["create", "new"]))]
    Add {
        /// property name
        name: String,
        /// property enum value
        enum_value_name: String,
        /// property enum color
        enum_value_color: String,
        /// property enum style (e.g., bold or underline)
        enum_value_style: Option<String>,
    },
    /// Get parameter of enum value
    Get {
        /// property name
        property: String,
        /// property enum value
        enum_value_name: String,
        /// parameter (color or style)
        parameter: String,
    },
    /// Set color for a property enum value
    Set {
        /// property name
        name: String,
        /// property enum value
        enum_value_name: String,
        /// property enum color
        enum_value_color: String,
        /// property enum color
        enum_value_style: Option<String>,
    },
    /// Delete a property enum value
    #[clap(visible_aliases(["del", "remove", "rem"]))]
    Delete {
        /// property name
        name: String,
        /// property enum value
        enum_value_name: String,
    },
}

fn main() -> ExitCode {
    let _ = enable_ansi_support::enable_ansi_support();
    let args = Args::parse();
    let success = match args.command {
        Some(Command::List { status, keyword, from, until, author, columns, sort, limit, no_color }) => task_list(status, keyword, from, until, author, columns, sort, limit, no_color),
        Some(Command::Show { id, no_color }) => task_show(id, no_color),
        Some(Command::Create { name, description, no_desc, push, remote }) => task_create(name, description, no_desc, push, &remote),
        Some(Command::Status { ids, status, push, remote, no_color }) => task_status(ids, status, push, &remote, no_color),
        Some(Command::Get { id, prop_name }) => task_get(id, prop_name),
        Some(Command::Set { id, prop_name, value, push, remote, no_color }) => task_set(id, prop_name, value, push, &remote, no_color),
        Some(Command::Unset { ids, prop_name }) => task_unset(ids, prop_name),
        Some(Command::Edit { id, prop_name }) => task_edit(id, prop_name),
        Some(Command::Comment { subcommand }) => task_comment(subcommand),
        Some(Command::Import { ids, format }) => task_import(ids, format),
        Some(Command::Export { ids, status, limit, format, pretty }) => task_export(ids, status, limit, format, pretty),
        Some(Command::Pull { ids, limit, status, remote, no_comments }) => task_pull(ids, limit, status, &remote, no_comments),
        Some(Command::Push { ids, remote, no_comments, no_color }) => task_push(ids, &remote, no_comments, no_color),
        Some(Command::Stats { no_color }) => task_stats(no_color),
        Some(Command::Delete { ids, status, push, remote }) => task_delete(ids, status, push, &remote),
        Some(Command::Clear) => task_clear(),
        Some(Command::Config { subcommand }) => task_config(subcommand),
        None => false
    };
    if success { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn task_comment(subcommand: CommentCommand) -> bool {
    match subcommand {
        CommentCommand::Add { task_id, text, push, remote } => task_comment_add(task_id, text, push, &remote),
        CommentCommand::Edit { task_id, comment_id, push, remote } => task_comment_edit(task_id, comment_id, push, &remote),
        CommentCommand::Delete { task_id, comment_id, push, remote } => task_comment_delete(task_id, comment_id, push, &remote),
    }
}

fn task_config(subcommand: ConfigCommand) -> bool {
    match subcommand {
        ConfigCommand::Get { param } => task_config_get(param),
        ConfigCommand::Set { param, value, move_ref } => task_config_set(param, value, move_ref),
        ConfigCommand::List => task_config_list(),
        ConfigCommand::Status { subcommand } => task_config_status(subcommand),
        ConfigCommand::Properties { subcommand } => task_config_properties(subcommand),
    }
}

fn task_config_status(subcommand: StatusCommand) -> bool {
    match subcommand {
        StatusCommand::Add { name, shortcut, color, is_done } => task_config_status_add(name, shortcut, color, is_done),
        StatusCommand::Delete { name, force } => task_config_status_delete(name, force),
        StatusCommand::Get { name, param } => task_config_status_get(name, param),
        StatusCommand::Set { name, param, value } => task_config_status_set(name, param, value),
        StatusCommand::List => task_config_status_list(),
        StatusCommand::Import => task_config_status_import(),
        StatusCommand::Export { pretty } => task_config_status_export(pretty),
        StatusCommand::Reset => task_config_status_reset(),
    }
}

fn task_config_properties(subcommand: PropertiesCommand) -> bool {
    match subcommand {
        PropertiesCommand::Add { name, value_type, color, enum_values } => task_config_properties_add(name, value_type, color, enum_values),
        PropertiesCommand::Delete { name, force } => task_config_properties_delete(name, force),
        PropertiesCommand::Get { name, param } => task_config_properties_get(name, param),
        PropertiesCommand::Set { name, param, value } => task_config_properties_set(name, param, value),
        PropertiesCommand::Enum { subcommand } => task_config_properties_enum(subcommand),
        PropertiesCommand::List => task_config_properties_list(),
        PropertiesCommand::Import => task_config_properties_import(),
        PropertiesCommand::Export { pretty } => task_config_properties_export(pretty),
        PropertiesCommand::Reset => task_config_properties_reset(),
    }
}

fn task_config_properties_enum(subcommand: PropertiesEnumCommand) -> bool {
    match subcommand {
        PropertiesEnumCommand::List { name } => task_config_properties_enum_list(name),
        PropertiesEnumCommand::Add { name, enum_value_name, enum_value_color, enum_value_style } => task_config_properties_enum_add(name, enum_value_name, enum_value_color, enum_value_style),
        PropertiesEnumCommand::Get { property, enum_value_name, parameter } => task_config_properties_enum_get(property, enum_value_name, parameter),
        PropertiesEnumCommand::Set { name, enum_value_name, enum_value_color, enum_value_style } => task_config_properties_enum_set(name, enum_value_name, enum_value_color, enum_value_style),
        PropertiesEnumCommand::Delete { name, enum_value_name } => task_config_properties_enum_delete(name, enum_value_name),
    }
}