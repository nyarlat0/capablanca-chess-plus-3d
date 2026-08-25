use crate::plan::{PlanState, StrategicPlan, generate_plan_candidates};
use capablanca_chess_plus::{Move, Position};
use terastockfish::{SearchHistory, SearchLimits, SearchOptions, Searcher, is_mate_score};

#[derive(Clone, Copy, Debug)]
pub struct HybridOptions {
    pub safety_nodes: u64,
    pub hash_megabytes: usize,
    pub search_threads: usize,
    pub ordinary_tolerance_cp: i32,
    pub expressive_tolerance_cp: i32,
    pub expressive_plan_confidence: f32,
    pub maximum_wdl_drop: f32,
    pub policy_candidates: usize,
}

impl Default for HybridOptions {
    fn default() -> Self {
        Self {
            safety_nodes: 50_000,
            hash_megabytes: 32,
            search_threads: 1,
            ordinary_tolerance_cp: 120,
            expressive_tolerance_cp: 300,
            expressive_plan_confidence: 0.75,
            maximum_wdl_drop: 0.08,
            policy_candidates: 8,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PolicyCandidate {
    pub chess_move: Move,
    pub probability: f32,
    /// Expected score `(win + draw/2)` after this move, from the root side's
    /// perspective. It is used only to decide whether an expressive sacrifice
    /// may receive the wider tactical tolerance.
    pub expected_score: f32,
}

#[derive(Clone, Debug)]
pub struct NeuralAdvice {
    pub policy: Vec<PolicyCandidate>,
    pub wdl: [f32; 3],
}

pub trait NeuralAdvisor {
    fn set_recent_moves(&mut self, _history: &[Move]) {}

    fn propose_plan(
        &mut self,
        position: &Position,
        candidates: &[StrategicPlan],
        previous: Option<&StrategicPlan>,
    ) -> Result<StrategicPlan, String>;

    fn evaluate(
        &mut self,
        position: &Position,
        plan: &StrategicPlan,
    ) -> Result<NeuralAdvice, String>;
}

#[derive(Clone, Debug)]
pub struct HybridDecision {
    pub chess_move: Move,
    pub plan: StrategicPlan,
    pub wdl: [f32; 3],
    pub safety_vetoed_policy_best: bool,
    pub safety_score: i32,
    pub tactical_best_score: i32,
    pub safety_nodes: u64,
    pub tactical_best: Move,
    pub plan_transition: PlanTransition,
    pub plan_cancelled_by_safety: bool,
    pub cancelled_plan_duration: Option<u8>,
    pub explanation_ru: String,
    pub explanation_en: String,
}

/// What happened to the persistent plan when this move was selected.
///
/// Durations count the agent's own moves spent on the previous plan. They are
/// exposed primarily for reproducible arena studies rather than move choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanTransition {
    Started,
    Continued,
    Completed { duration: u8 },
    HorizonExpired { duration: u8 },
    BecameInapplicable { duration: u8 },
    Superseded { duration: u8 },
}

impl PlanTransition {
    #[must_use]
    pub const fn ended_duration(self) -> Option<u8> {
        match self {
            Self::Completed { duration }
            | Self::HorizonExpired { duration }
            | Self::BecameInapplicable { duration }
            | Self::Superseded { duration } => Some(duration),
            Self::Started | Self::Continued => None,
        }
    }
}

pub struct HybridAgent<A> {
    advisor: A,
    safety: Searcher,
    options: HybridOptions,
    plan_state: PlanState,
}

impl<A: NeuralAdvisor> HybridAgent<A> {
    #[must_use]
    pub fn new(advisor: A, options: HybridOptions) -> Self {
        let safety = Searcher::new(SearchOptions {
            hash_megabytes: options.hash_megabytes,
            threads: options.search_threads,
        });
        Self {
            advisor,
            safety,
            options,
            plan_state: PlanState::default(),
        }
    }

    #[must_use]
    pub const fn plan_state(&self) -> &PlanState {
        &self.plan_state
    }

    pub fn reset(&mut self) {
        self.plan_state = PlanState::default();
        self.safety.clear_hash();
    }

    pub fn set_safety_nodes(&mut self, nodes: u64) {
        self.options.safety_nodes = nodes.max(1);
    }

    pub fn resize_hash(&mut self, megabytes: usize) {
        self.options.hash_megabytes = megabytes.max(1);
        self.safety.resize_hash(self.options.hash_megabytes);
    }

    pub fn set_search_threads(&mut self, threads: usize) {
        self.options.search_threads = threads.max(1);
        self.safety.set_threads(self.options.search_threads);
    }

    pub fn decide(
        &mut self,
        position: &Position,
        history: Option<&SearchHistory>,
    ) -> Result<HybridDecision, String> {
        self.decide_with_recent_moves(position, history, &[])
    }

    pub fn decide_with_recent_moves(
        &mut self,
        position: &Position,
        history: Option<&SearchHistory>,
        recent_moves: &[Move],
    ) -> Result<HybridDecision, String> {
        let legal = position.legal_moves();
        if legal.is_empty() {
            return Err("cannot choose a move in a terminal position".to_owned());
        }
        let plans = generate_plan_candidates(position);
        if plans.is_empty() {
            return Err("symbolic planner produced no applicable plan".to_owned());
        }
        self.advisor.set_recent_moves(recent_moves);
        let proposed =
            self.advisor
                .propose_plan(position, &plans, self.plan_state.current.as_ref())?;
        if !plans
            .iter()
            .any(|candidate| same_plan(candidate, &proposed))
        {
            return Err(
                "neural planner selected a plan outside the applicable candidate set".to_owned(),
            );
        }
        let plan_transition = plan_transition(&self.plan_state, position, &proposed);
        let plan = self.plan_state.choose(position, proposed).clone();
        let mut advice = self.advisor.evaluate(position, &plan)?;
        validate_advice(&advice, &legal)?;
        advice.policy.sort_by(|left, right| {
            right
                .probability
                .total_cmp(&left.probability)
                .then_with(|| left.chess_move.to_uci().cmp(&right.chess_move.to_uci()))
        });
        advice
            .policy
            .truncate(self.options.policy_candidates.max(1));

        let limits = SearchLimits {
            max_nodes: Some(self.options.safety_nodes.max(1)),
            ..SearchLimits::depth(16)
        };
        let analysis = match history {
            Some(history) => self.safety.analyze_with_history(position, history, limits),
            None => self.safety.analyze(position, limits),
        };
        let tactical_best = analysis.best_move.ok_or_else(|| {
            "safety search returned no move for a non-terminal position".to_owned()
        })?;
        let best_score = analysis.score;
        let policy_best = advice.policy[0].chess_move;
        let root_expected = advice.wdl[0] + advice.wdl[1] * 0.5;
        let chosen = advice
            .policy
            .iter()
            .find(|candidate| {
                let score = analysis
                    .root_candidates
                    .iter()
                    .find(|root| root.chess_move == candidate.chess_move)
                    .map_or_else(|| i32::MIN, |root| root.score);
                candidate_is_safe(
                    score,
                    best_score,
                    candidate.expected_score,
                    root_expected,
                    plan.confidence,
                    self.options,
                )
            })
            .map_or(tactical_best, |candidate| candidate.chess_move);
        let veto = chosen != policy_best;
        self.plan_state.record_chosen_move(chosen);
        let current_plan_duration = self.plan_state.own_moves_elapsed;
        if veto {
            self.plan_state.record_safety_veto();
        } else {
            self.plan_state.record_safe_choice();
        }
        let plan_cancelled_by_safety = veto && self.plan_state.current.is_none();
        let cancelled_plan_duration = plan_cancelled_by_safety.then_some(current_plan_duration);
        let safety_score = analysis
            .root_candidates
            .iter()
            .find(|candidate| candidate.chess_move == chosen)
            .map_or(best_score, |candidate| candidate.score);
        Ok(HybridDecision {
            chess_move: chosen,
            explanation_ru: plan.text_ru(),
            explanation_en: plan.text_en(),
            plan,
            wdl: advice.wdl,
            safety_vetoed_policy_best: veto,
            safety_score,
            tactical_best_score: best_score,
            safety_nodes: analysis.nodes,
            tactical_best,
            plan_transition,
            plan_cancelled_by_safety,
            cancelled_plan_duration,
        })
    }
}

fn plan_transition(
    state: &PlanState,
    position: &Position,
    proposed: &StrategicPlan,
) -> PlanTransition {
    let Some(current) = state.current.as_ref() else {
        return PlanTransition::Started;
    };
    if !current.applicable(position) {
        return PlanTransition::BecameInapplicable {
            duration: state.own_moves_elapsed,
        };
    }
    if state.own_moves_elapsed >= current.horizon {
        return PlanTransition::HorizonExpired {
            duration: state.own_moves_elapsed,
        };
    }
    if current.measured_progress(position) >= 0.85 {
        return PlanTransition::Completed {
            duration: state.own_moves_elapsed,
        };
    }
    if proposed.confidence >= current.confidence + 0.15 {
        return PlanTransition::Superseded {
            duration: state.own_moves_elapsed,
        };
    }
    PlanTransition::Continued
}

fn same_plan(left: &StrategicPlan, right: &StrategicPlan) -> bool {
    left.side == right.side
        && left.kind == right.kind
        && left.actor == right.actor
        && left.target == right.target
        && left.method == right.method
        && left.reason == right.reason
}

fn validate_advice(advice: &NeuralAdvice, legal: &[Move]) -> Result<(), String> {
    if advice.policy.is_empty() {
        return Err("neural policy is empty".to_owned());
    }
    if advice.policy.iter().any(|candidate| {
        !legal.contains(&candidate.chess_move)
            || !candidate.probability.is_finite()
            || candidate.probability < 0.0
            || !candidate.expected_score.is_finite()
            || !(0.0..=1.0).contains(&candidate.expected_score)
    }) {
        return Err("neural policy contains an invalid or illegal candidate".to_owned());
    }
    let probability = advice
        .policy
        .iter()
        .map(|candidate| candidate.probability)
        .sum::<f32>();
    if (probability - 1.0).abs() > 1e-3 {
        return Err(format!(
            "neural policy probabilities sum to {probability}, not one"
        ));
    }
    let wdl = advice.wdl.iter().sum::<f32>();
    if advice
        .wdl
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        || (wdl - 1.0).abs() > 1e-3
    {
        return Err("neural WDL must be finite, non-negative, and normalized".to_owned());
    }
    Ok(())
}

fn candidate_is_safe(
    candidate_score: i32,
    best_score: i32,
    candidate_expected: f32,
    root_expected: f32,
    plan_confidence: f32,
    options: HybridOptions,
) -> bool {
    if candidate_score == i32::MIN {
        return false;
    }
    if candidate_score < 0 && is_mate_score(candidate_score) && !is_mate_score(best_score) {
        return false;
    }
    let expressive = plan_confidence >= options.expressive_plan_confidence
        && root_expected - candidate_expected <= options.maximum_wdl_drop;
    let tolerance = if expressive {
        options.expressive_tolerance_cp
    } else {
        options.ordinary_tolerance_cp
    };
    best_score.saturating_sub(candidate_score) <= tolerance
}

#[cfg(test)]
mod tests {
    use super::*;
    use capablanca_chess_plus::Variant;

    struct DeliberatelyBadAdvisor;

    impl NeuralAdvisor for DeliberatelyBadAdvisor {
        fn propose_plan(
            &mut self,
            _position: &Position,
            candidates: &[StrategicPlan],
            _previous: Option<&StrategicPlan>,
        ) -> Result<StrategicPlan, String> {
            Ok(candidates[0].clone())
        }

        fn evaluate(
            &mut self,
            position: &Position,
            _plan: &StrategicPlan,
        ) -> Result<NeuralAdvice, String> {
            let mut moves = position.legal_moves();
            moves.sort_unstable_by_key(|chess_move| std::cmp::Reverse(chess_move.to_uci()));
            let probability = 1.0 / moves.len() as f32;
            Ok(NeuralAdvice {
                policy: moves
                    .into_iter()
                    .map(|chess_move| PolicyCandidate {
                        chess_move,
                        probability,
                        expected_score: 0.5,
                    })
                    .collect(),
                wdl: [0.25, 0.5, 0.25],
            })
        }
    }

    #[test]
    fn hybrid_decision_is_legal_and_explained() {
        let position = Variant::TerachessII.starting_position();
        let mut agent = HybridAgent::new(
            DeliberatelyBadAdvisor,
            HybridOptions {
                safety_nodes: 1_000,
                hash_megabytes: 1,
                ..HybridOptions::default()
            },
        );
        let decision = agent.decide(&position, None).unwrap();
        assert!(position.legal_moves().contains(&decision.chess_move));
        assert!(decision.explanation_ru.contains("Цель"));
        assert!(decision.explanation_en.contains("Goal"));
    }

    #[test]
    fn forced_mate_loss_cannot_use_the_expressive_tolerance() {
        let options = HybridOptions::default();
        assert!(!candidate_is_safe(-999_999, 100, 0.7, 0.7, 1.0, options));
    }
}
