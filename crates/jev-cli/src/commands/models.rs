//! `jev models list`.

use jev_client::ModelCard;
use serde::Serialize;

use super::{Context, eval};
use crate::error::CliError;
use crate::output::{Cell, Render, Table, Ui};

pub(crate) fn list(context: &mut Context<'_>) -> Result<(), CliError> {
    let settings = context.settings()?.clone();
    let transport = context.transport(&settings)?;
    let models = eval::list_models(&transport)?;
    context.output.emit(
        &Models {
            models: models.models,
        },
        context.stdout,
    )
}

#[derive(Debug, Serialize)]
struct Models {
    models: Vec<ModelCard>,
}

impl Render for Models {
    fn human(&self, ui: Ui) -> String {
        if self.models.is_empty() {
            return "no models are listed for this account\n".to_owned();
        }
        let mut table = Table::default();
        table.row(
            ["NAME", "RELEASED", "DESCRIPTION"]
                .map(|title| Cell::styled(title, ui.dim(title)))
                .into(),
        );
        for model in &self.models {
            // The API sends a full timestamp; the date is what a person wants.
            let released = model
                .release_date
                .as_deref()
                .map_or("-", |date| date.split('T').next().unwrap_or(date));
            table.row(vec![
                Cell::styled(&model.name, ui.bold(&model.name)),
                Cell::plain(released),
                Cell::plain(model.description.as_deref().unwrap_or("-")),
            ]);
        }
        table.render("")
    }

    fn records(&self) -> Option<Vec<serde_json::Value>> {
        Some(
            self.models
                .iter()
                .filter_map(|model| serde_json::to_value(model).ok())
                .collect(),
        )
    }
}
