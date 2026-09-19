//! `jev schema`: the JSON Schemas of what `jev` reads and writes.

use serde::Serialize;
use serde_json::Value;

use super::Context;
use crate::cli::SchemaCommand;
use crate::error::CliError;
use crate::output::{Render, Ui};
use crate::schemas;

pub(crate) fn run(command: &SchemaCommand, context: &mut Context<'_>) -> Result<(), CliError> {
    let schema = match command {
        SchemaCommand::Request => schemas::request(),
        SchemaCommand::Questions => schemas::questions(),
        SchemaCommand::Output => schemas::output(),
        SchemaCommand::Error => schemas::error(),
        SchemaCommand::BatchRecord => schemas::batch_record(),
    };
    context.output.emit(&Schema(schema), context.stdout)
}

/// A schema, printed as it is.
#[derive(Serialize)]
#[serde(transparent)]
struct Schema(Value);

impl Render for Schema {
    /// A schema is a JSON document, so a person gets it as JSON too, only indented.
    fn human(&self, _: Ui) -> String {
        let mut text = serde_json::to_string_pretty(&self.0).unwrap_or_default();
        text.push('\n');
        text
    }
}
