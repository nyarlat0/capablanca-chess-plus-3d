use crate::EvaluationParameters;
use capablanca_chess_plus::PieceKind;

pub const MATERIAL_PROFILE_VERSION: &str = "TERASTOCKFISH_MATERIAL_PROFILE_V1";

/// A named, portable material profile. Positional terms come from the
/// published evaluator so the file describes exactly the values tuned by the
/// Texel pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterialProfile {
    pub name: String,
    pub parameters: EvaluationParameters,
}

impl MaterialProfile {
    #[must_use]
    pub fn new(name: impl Into<String>, parameters: EvaluationParameters) -> Self {
        Self {
            name: name.into(),
            parameters,
        }
    }

    #[must_use]
    pub fn encode(&self) -> String {
        let mut output = format!(
            "{MATERIAL_PROFILE_VERSION}\nname={}\nbase=published\n",
            self.name
        );
        for kind in PieceKind::ALL {
            output.push_str(&format!(
                "material.{}={}\n",
                piece_name(kind),
                self.parameters.material_value(kind)
            ));
        }
        output
    }

    pub fn decode(contents: &str) -> Result<Self, String> {
        let mut lines = contents.lines();
        if lines.next() != Some(MATERIAL_PROFILE_VERSION) {
            return Err("unsupported or corrupt material profile".to_owned());
        }
        let mut name = None;
        let mut base = None;
        let mut parameters = EvaluationParameters::published();
        let mut seen = [false; PieceKind::COUNT];
        for line in lines {
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("invalid profile line `{line}`"));
            };
            match key {
                "name" => {
                    if value.is_empty() || name.replace(value.to_owned()).is_some() {
                        return Err("profile must contain one non-empty name".to_owned());
                    }
                }
                "base" => {
                    if value != "published" || base.replace(value).is_some() {
                        return Err("material profile base must be `published`".to_owned());
                    }
                }
                _ => {
                    let piece = key
                        .strip_prefix("material.")
                        .ok_or_else(|| format!("unknown profile field `{key}`"))?;
                    let kind = parse_piece_kind(piece)?;
                    if seen[kind.index()] {
                        return Err(format!("duplicate material value for `{piece}`"));
                    }
                    let value = value
                        .parse::<i32>()
                        .map_err(|_| format!("invalid material value `{value}`"))?;
                    if value < 0 {
                        return Err("material values cannot be negative".to_owned());
                    }
                    parameters.set_material_value(kind, value);
                    seen[kind.index()] = true;
                }
            }
        }
        if base.is_none() || seen.iter().any(|present| !present) {
            return Err("material profile is incomplete".to_owned());
        }
        if parameters.material_value(PieceKind::Pawn) != 100
            || parameters.material_value(PieceKind::King) != 0
        {
            return Err("material profile must anchor pawn at 100 and king at 0".to_owned());
        }
        Ok(Self {
            name: name.ok_or_else(|| "material profile lacks a name".to_owned())?,
            parameters,
        })
    }
}

#[must_use]
pub const fn piece_name(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::Pawn => "pawn",
        PieceKind::Knight => "knight",
        PieceKind::Bishop => "bishop",
        PieceKind::Rook => "rook",
        PieceKind::Queen => "queen",
        PieceKind::King => "king",
        PieceKind::Archbishop => "archbishop",
        PieceKind::Chancellor => "chancellor",
        PieceKind::Cannon => "cannon",
        PieceKind::Elephant => "elephant",
        PieceKind::Camel => "camel",
        PieceKind::Giraffe => "giraffe",
        PieceKind::Archer => "archer",
        PieceKind::Machine => "machine",
        PieceKind::Amazon => "amazon",
        PieceKind::Lion => "lion",
        PieceKind::Buffalo => "buffalo",
        PieceKind::Centaur => "centaur",
        PieceKind::Admiral => "admiral",
        PieceKind::Missionary => "missionary",
        PieceKind::Eagle => "eagle",
        PieceKind::Rhinoceros => "rhinoceros",
        PieceKind::Prince => "prince",
        PieceKind::Sorceress => "sorceress",
        PieceKind::Duchess => "duchess",
        PieceKind::Troll => "troll",
    }
}

pub fn parse_piece_kind(name: &str) -> Result<PieceKind, String> {
    PieceKind::ALL
        .into_iter()
        .find(|kind| piece_name(*kind) == name)
        .ok_or_else(|| format!("unknown piece kind `{name}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_profile_round_trips_every_piece() {
        let mut parameters = EvaluationParameters::published();
        parameters.set_material_value(PieceKind::Archer, 438);
        let profile = MaterialProfile::new("candidate", parameters);
        assert_eq!(MaterialProfile::decode(&profile.encode()).unwrap(), profile);
    }

    #[test]
    fn material_profile_rejects_missing_values_and_changed_anchors() {
        assert!(
            MaterialProfile::decode(
                "TERASTOCKFISH_MATERIAL_PROFILE_V1\nname=x\nbase=published\nmaterial.pawn=100\n"
            )
            .is_err()
        );

        let changed = MaterialProfile::new("bad", {
            let mut parameters = EvaluationParameters::published();
            parameters.set_material_value(PieceKind::Pawn, 99);
            parameters
        });
        assert!(MaterialProfile::decode(&changed.encode()).is_err());
    }
}
