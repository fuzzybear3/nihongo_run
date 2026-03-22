use bevy::prelude::Resource;
use rand::{Rng, SeedableRng, rngs::SmallRng};

// (hiragana reading, kanji/kana display)
pub const N5_WORDS: &[(&str, &str)] = &[
    ("みず", "水"),
    ("ひ", "火"),
    ("やま", "山"),
    ("かわ", "川"),
    ("き", "木"),
    ("はな", "花"),
    ("いぬ", "犬"),
    ("ねこ", "猫"),
    ("さかな", "魚"),
    ("とり", "鳥"),
    ("たべる", "食べる"),
    ("のむ", "飲む"),
    ("いく", "行く"),
    ("くる", "来る"),
    ("みる", "見る"),
    ("きく", "聞く"),
    ("はなす", "話す"),
    ("かく", "書く"),
    ("よむ", "読む"),
    ("かう", "買う"),
    ("おおきい", "大きい"),
    ("ちいさい", "小さい"),
    ("あたらしい", "新しい"),
    ("ふるい", "古い"),
    ("たかい", "高い"),
    ("やすい", "安い"),
    ("しろい", "白い"),
    ("くろい", "黒い"),
    ("あかい", "赤い"),
    ("あおい", "青い"),
];

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
    pub cards: Vec<CardState>,
    pub gate_pass_count: u32,
    pub rng: SmallRng,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            cards: (0..N5_WORDS.len()).map(|_| CardState::default()).collect(),
            gate_pass_count: 0,
            rng: SmallRng::seed_from_u64(42),
        }
    }

    /// Returns (kanji_display, correct_hiragana, distractor_hiragana, word_index).
    pub fn pick(&mut self) -> (&'static str, &'static str, &'static str, usize) {
        let due: Vec<usize> = (0..self.cards.len())
            .filter(|&i| self.cards[i].due_at <= self.gate_pass_count)
            .collect();

        let q = if !due.is_empty() {
            let i = self.rng.gen_range(0..due.len());
            due[i]
        } else {
            // No cards due yet — pick the soonest upcoming one
            (0..self.cards.len())
                .min_by_key(|&i| self.cards[i].due_at)
                .unwrap_or(0)
        };

        let mut d = self.rng.gen_range(0..N5_WORDS.len() - 1);
        if d >= q { d += 1; }

        (N5_WORDS[q].1, N5_WORDS[q].0, N5_WORDS[d].0, q)
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
