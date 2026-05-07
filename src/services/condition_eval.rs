use crate::models::condition::TikTokConditions;

#[derive(Debug, Clone)]
pub struct CacheData {
    pub is_verified: bool,
    pub follower_count: i64,
    pub following_count: i64,
    pub likes_count: i64,
    pub video_count: i64,
    pub has_bio: bool,
    pub bio_length: i32,
}

/// Evaluate whether a user meets the configured conditions.
/// Pure, synchronous, fast — no I/O, no allocations.
/// If no toggles are enabled the role grants to any linked TikTok account
/// (consistent with the schema's user-facing description).
pub fn evaluate(conditions: &TikTokConditions, cache: &CacheData) -> bool {
    if conditions.require_verified && !cache.is_verified {
        return false;
    }
    if conditions.require_followers {
        if cache.follower_count < conditions.min_followers {
            return false;
        }
    }
    if conditions.require_following && cache.following_count < conditions.min_following {
        return false;
    }
    if conditions.require_likes && cache.likes_count < conditions.min_likes {
        return false;
    }
    if conditions.require_videos && cache.video_count < conditions.min_videos {
        return false;
    }
    if conditions.require_bio && (!cache.has_bio || cache.bio_length < conditions.min_bio_length) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rich_cache() -> CacheData {
        CacheData {
            is_verified: true,
            follower_count: 50_000,
            following_count: 200,
            likes_count: 1_000_000,
            video_count: 100,
            has_bio: true,
            bio_length: 40,
        }
    }

    #[test]
    fn empty_conditions_grant_anyone() {
        let conditions = TikTokConditions::default();
        assert!(evaluate(&conditions, &rich_cache()));
    }

    #[test]
    fn require_verified_passes_when_verified() {
        let conditions = TikTokConditions {
            require_verified: true,
            ..Default::default()
        };
        assert!(evaluate(&conditions, &rich_cache()));
    }

    #[test]
    fn require_verified_fails_when_not_verified() {
        let conditions = TikTokConditions {
            require_verified: true,
            ..Default::default()
        };
        let mut cache = rich_cache();
        cache.is_verified = false;
        assert!(!evaluate(&conditions, &cache));
    }

    #[test]
    fn min_followers_passes() {
        let conditions = TikTokConditions {
            require_followers: true,
            min_followers: 1000,
            ..Default::default()
        };
        assert!(evaluate(&conditions, &rich_cache()));
    }

    #[test]
    fn min_followers_fails_too_low() {
        let conditions = TikTokConditions {
            require_followers: true,
            min_followers: 100_000,
            ..Default::default()
        };
        assert!(!evaluate(&conditions, &rich_cache()));
    }

    #[test]
    fn min_following_fails_too_low() {
        let conditions = TikTokConditions {
            require_following: true,
            min_following: 1000,
            ..Default::default()
        };
        // rich_cache has following_count = 200
        assert!(!evaluate(&conditions, &rich_cache()));
    }

    #[test]
    fn min_likes_passes() {
        let conditions = TikTokConditions {
            require_likes: true,
            min_likes: 100_000,
            ..Default::default()
        };
        assert!(evaluate(&conditions, &rich_cache()));
    }

    #[test]
    fn min_videos_fails_too_low() {
        let conditions = TikTokConditions {
            require_videos: true,
            min_videos: 1000,
            ..Default::default()
        };
        assert!(!evaluate(&conditions, &rich_cache()));
    }

    #[test]
    fn require_bio_fails_when_empty() {
        let conditions = TikTokConditions {
            require_bio: true,
            min_bio_length: 1,
            ..Default::default()
        };
        let mut cache = rich_cache();
        cache.has_bio = false;
        cache.bio_length = 0;
        assert!(!evaluate(&conditions, &cache));
    }

    #[test]
    fn require_bio_fails_when_too_short() {
        let conditions = TikTokConditions {
            require_bio: true,
            min_bio_length: 80,
            ..Default::default()
        };
        // rich_cache has bio_length = 40
        assert!(!evaluate(&conditions, &rich_cache()));
    }

    #[test]
    fn combined_conditions_all_pass() {
        let conditions = TikTokConditions {
            require_verified: true,
            require_followers: true,
            min_followers: 10_000,
            require_videos: true,
            min_videos: 50,
            require_bio: true,
            min_bio_length: 10,
            ..Default::default()
        };
        assert!(evaluate(&conditions, &rich_cache()));
    }

    #[test]
    fn combined_conditions_one_fails_overall_fails() {
        let conditions = TikTokConditions {
            require_verified: true,
            require_followers: true,
            min_followers: 10_000,
            require_bio: true,
            min_bio_length: 200, // too long
            ..Default::default()
        };
        assert!(!evaluate(&conditions, &rich_cache()));
    }
}
