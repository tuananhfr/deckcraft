//! Source content that the importer cannot carry into a saved presentation.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportLosses {
    pub signatures: bool,
    pub macros: bool,
    pub parts: Vec<String>,
}

impl ImportLosses {
    pub fn is_empty(&self) -> bool {
        !self.signatures && !self.macros && self.parts.is_empty()
    }
}
