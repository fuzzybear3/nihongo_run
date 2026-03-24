use bevy::prelude::Resource;
use rand::{Rng, SeedableRng, rngs::SmallRng};

pub struct CardState {
    pub interval: f32, // gate-passes until next review
    pub ease: f32,     // interval multiplier (default 2.5)
    pub reps: u32,     // consecutive correct answers
    pub due_at: u32,   // gate_pass_count when this card is next due
}

impl Default for CardState {
    fn default() -> Self {
        Self { interval: 0.0, ease: 2.5, reps: 0, due_at: 0 }
    }
}

#[derive(Resource)]
pub struct Scheduler {
    /// `(hiragana, display)` — display is kanji when available, else hiragana.
    pub words: Vec<(String, String)>,
    pub cards: Vec<CardState>,
    pub gate_pass_count: u32,
    pub rng: SmallRng,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            words: Vec::new(),
            cards: Vec::new(),
            gate_pass_count: 0,
            rng: SmallRng::seed_from_u64(42),
        }
    }

    /// Called once words arrive from Supabase. Resets all card state.
    pub fn load_words(&mut self, words: Vec<(String, String)>) {
        let n = words.len();
        self.words = words;
        self.cards = (0..n).map(|_| CardState::default()).collect();
    }

    pub fn is_ready(&self) -> bool {
        !self.words.is_empty()
    }

    /// Returns `(display, correct_hiragana, distractor_hiragana, word_index)`.
    pub fn pick(&mut self) -> (String, String, String, usize) {
        let n = self.words.len();
        let due: Vec<usize> = (0..n)
            .filter(|&i| self.cards[i].due_at <= self.gate_pass_count)
            .collect();

        let q = if !due.is_empty() {
            due[self.rng.gen_range(0..due.len())]
        } else {
            (0..n).min_by_key(|&i| self.cards[i].due_at).unwrap_or(0)
        };

        let mut d = self.rng.gen_range(0..n - 1);
        if d >= q { d += 1; }

        (
            self.words[q].1.clone(),
            self.words[q].0.clone(),
            self.words[d].0.clone(),
            q,
        )
    }

    /// Update the SM-2 state for `word_index` based on whether the answer was correct.
    pub fn record(&mut self, word_index: usize, correct: bool) {
        let card = &mut self.cards[word_index];
        if correct {
            card.interval = if card.reps == 0 {
                1.0
            } else if card.reps == 1 {
                6.0
            } else {
                (card.interval * card.ease).round()
            };
            card.reps += 1;
            card.ease = (card.ease + 0.1).min(3.0);
        } else {
            card.interval = 1.0;
            card.reps = 0;
            card.ease = (card.ease - 0.2).max(1.3);
        }
        card.due_at = self.gate_pass_count + card.interval as u32;
    }
}
