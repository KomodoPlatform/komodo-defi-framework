use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum TaskSubcommand {
    #[command(subcommand, about = "Get status of task")]
    Status(TaskSubcommandStatus),
    #[command(subcommand, about = "Cancel task")]
    Cancel(TaskSubcommandCancel),
}

#[derive(Subcommand)]
pub(crate) enum TaskSubcommandStatus {
    #[command(about = "Get zcoin enabling status")]
    Zcoin { task_id: u64 },
}

#[derive(Subcommand)]
pub(crate) enum TaskSubcommandCancel {
    #[command(about = "Cancel enabling zcoin")]
    Zcoin { task_id: u64 },
}
