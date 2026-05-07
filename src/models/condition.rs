use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TikTokConditions {
    #[serde(default)]
    pub require_verified: bool,

    #[serde(default)]
    pub require_followers: bool,
    #[serde(default)]
    pub min_followers: i64,

    #[serde(default)]
    pub require_following: bool,
    #[serde(default)]
    pub min_following: i64,

    #[serde(default)]
    pub require_likes: bool,
    #[serde(default)]
    pub min_likes: i64,

    #[serde(default)]
    pub require_videos: bool,
    #[serde(default)]
    pub min_videos: i64,

    #[serde(default)]
    pub require_bio: bool,
    #[serde(default)]
    pub min_bio_length: i32,
}
