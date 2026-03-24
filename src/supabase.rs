use serde::Deserialize;

const URL: &str = "https://vxasarkxxperwuqsanne.supabase.co";
const ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.\
    eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6InZ4YXNhcmt4eHBlcnd1cXNhbm5lIiwicm9sZSI6ImFub24iLCJpYXQiOjE3NzQzMTg1NjUsImV4cCI6MjA4OTg5NDU2NX0.\
    c7QD-Dt487CJUXzwKMhOKvhFOR36xXdforQqb6bZ6kc";

#[derive(Deserialize)]
struct Row {
    hiragana: String,
    kanji: Option<String>,
}

/// Fetches N5 words from Supabase.
/// Returns `(hiragana, display)` pairs where `display` is the kanji form
/// when available, otherwise hiragana.
/// Logs an error and returns an empty Vec on failure.
pub async fn fetch_words() -> Vec<(String, String)> {
    // On native, reqwest/hyper requires a Tokio runtime context.
    // async-compat::Compat provides one without needing a full Tokio app.
    #[cfg(not(target_arch = "wasm32"))]
    let result = async_compat::Compat::new(fetch_inner()).await;
    #[cfg(target_arch = "wasm32")]
    let result = fetch_inner().await;

    match result {
        Ok(words) => words,
        Err(e) => {
            bevy::log::error!("supabase fetch failed: {e}");
            Vec::new()
        }
    }
}

async fn fetch_inner() -> Result<Vec<(String, String)>, reqwest::Error> {
    let rows: Vec<Row> = reqwest::Client::new()
        .get(format!("{URL}/rest/v1/words?select=hiragana,kanji&jlpt_level=eq.N5&order=hiragana"))
        .header("apikey", ANON_KEY)
        .header("Authorization", format!("Bearer {ANON_KEY}"))
        .send()
        .await?
        .json()
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let display = r.kanji.unwrap_or_else(|| r.hiragana.clone());
            (r.hiragana, display)
        })
        .collect())
}
