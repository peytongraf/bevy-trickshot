//! Operators: who a player plays as. Picked in the lobby room for `Zombies`
//! ([`crate::SetOperator`], kept on [`crate::LobbyMember::operator`]). For now
//! all an operator changes is the voice — their quotes live under
//! `assets/audio/quotes/<operator>/` on the client.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Operator {
    #[default]
    Adam,
    Von,
}

impl Operator {
    /// In the order the lobby offers them, the default first.
    pub const ALL: [Operator; 2] = [Operator::Adam, Operator::Von];

    pub fn label(self) -> &'static str {
        match self {
            Operator::Adam => "ADAM",
            Operator::Von => "VON",
        }
    }

    /// Their folder name under `audio/quotes/` and `textures/operators/`.
    pub fn dir(self) -> &'static str {
        match self {
            Operator::Adam => "adam",
            Operator::Von => "von",
        }
    }
}
