use std::collections::HashMap;

use serde_json::{json, Value};

use crate::error::AppError;
use crate::models::condition::TikTokConditions;

pub fn build_config_schema(conditions: &TikTokConditions, verify_url: &str) -> Value {
    json!({
        "version": 1,
        "name": "TikTok Creator Role",
        "description": "Assign Discord roles based on TikTok account stats — followers, verification, videos, and engagement.",
        "sections": [
            {
                "title": "How it works",
                "fields": [
                    {
                        "type": "display",
                        "key": "intro",
                        "label": "Setup steps",
                        "value": "1. Toggle the conditions you want below — all enabled conditions must be met (AND logic).\n2. Share the verification link with members.\n3. Members link Discord and TikTok at the link → role granted automatically.\n4. TikTok stats refresh in the background (every 30 min – 24 hr per user)."
                    }
                ]
            },
            {
                "title": "Member verification link",
                "fields": [
                    {
                        "type": "url",
                        "key": "verify_url",
                        "label": "Share this link with your members",
                        "description": "Members visit this URL to link their Discord and TikTok accounts.",
                        "default_value": verify_url
                    }
                ]
            },
            {
                "title": "Account conditions",
                "description": "Toggle the requirements you want. Leave all toggles off to grant the role to anyone who links a TikTok account.",
                "fields": [
                    {
                        "type": "toggle",
                        "key": "require_verified",
                        "label": "Require verified account",
                        "description": "Member's TikTok account must have the verified (blue check) badge."
                    },

                    {
                        "type": "toggle",
                        "key": "require_followers",
                        "label": "Require minimum follower count",
                        "description": "Member must have at least the specified number of followers."
                    },
                    {
                        "type": "number",
                        "key": "min_followers",
                        "label": "Minimum followers",
                        "default_value": 1000,
                        "validation": { "min": 0 },
                        "condition": { "field": "require_followers", "equals": true }
                    },

                    {
                        "type": "toggle",
                        "key": "require_following",
                        "label": "Require minimum following count",
                        "description": "Member must follow at least N other accounts. Helps filter out throwaway accounts."
                    },
                    {
                        "type": "number",
                        "key": "min_following",
                        "label": "Minimum following",
                        "default_value": 50,
                        "validation": { "min": 0 },
                        "condition": { "field": "require_following", "equals": true }
                    },

                    {
                        "type": "toggle",
                        "key": "require_likes",
                        "label": "Require minimum total likes",
                        "description": "Sum of likes across all of the member's videos."
                    },
                    {
                        "type": "number",
                        "key": "min_likes",
                        "label": "Minimum total likes",
                        "default_value": 1000,
                        "validation": { "min": 0 },
                        "condition": { "field": "require_likes", "equals": true }
                    },

                    {
                        "type": "toggle",
                        "key": "require_videos",
                        "label": "Require minimum video count",
                        "description": "Member must have posted at least N videos."
                    },
                    {
                        "type": "number",
                        "key": "min_videos",
                        "label": "Minimum videos posted",
                        "default_value": 5,
                        "validation": { "min": 0 },
                        "condition": { "field": "require_videos", "equals": true }
                    },

                    {
                        "type": "toggle",
                        "key": "require_bio",
                        "label": "Require non-empty bio",
                        "description": "Helps filter out throwaway / spam accounts."
                    },
                    {
                        "type": "number",
                        "key": "min_bio_length",
                        "label": "Minimum bio length (characters)",
                        "default_value": 1,
                        "validation": { "min": 1, "max": 80 },
                        "condition": { "field": "require_bio", "equals": true }
                    }
                ]
            },
            {
                "title": "Common setups",
                "collapsible": true,
                "default_collapsed": true,
                "fields": [
                    {
                        "type": "display",
                        "key": "examples",
                        "label": "Recipes",
                        "value": "Verified creators only  \u{2192}  Enable 'Require verified account'\nInfluencer (10k+)  \u{2192}  Enable 'Require min followers', set to 10000\nMega creator (100k+)  \u{2192}  Enable 'Require min followers', set to 100000\nActive TikToker  \u{2192}  Enable 'Require videos', set to 20\nAnti-spam (real account)  \u{2192}  Enable 'Require bio' + 'Require videos' (min=3)"
                    }
                ]
            }
        ],
        "values": {
            "verify_url": verify_url,
            "require_verified": conditions.require_verified,
            "require_followers": conditions.require_followers,
            "min_followers": conditions.min_followers,
            "require_following": conditions.require_following,
            "min_following": conditions.min_following,
            "require_likes": conditions.require_likes,
            "min_likes": conditions.min_likes,
            "require_videos": conditions.require_videos,
            "min_videos": conditions.min_videos,
            "require_bio": conditions.require_bio,
            "min_bio_length": conditions.min_bio_length
        }
    })
}

fn get_bool(config: &HashMap<String, Value>, key: &str) -> bool {
    config.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn get_i64(config: &HashMap<String, Value>, key: &str, default: i64) -> i64 {
    config
        .get(key)
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .unwrap_or(default)
}

pub fn parse_config(config: &HashMap<String, Value>) -> Result<TikTokConditions, AppError> {
    let require_verified = get_bool(config, "require_verified");

    let require_followers = get_bool(config, "require_followers");
    let min_followers = if require_followers {
        get_i64(config, "min_followers", 0)
    } else {
        0
    };
    if min_followers < 0 {
        return Err(AppError::BadRequest(
            "Minimum followers must be 0 or greater".into(),
        ));
    }

    let require_following = get_bool(config, "require_following");
    let min_following = if require_following {
        get_i64(config, "min_following", 0)
    } else {
        0
    };
    if min_following < 0 {
        return Err(AppError::BadRequest(
            "Minimum following must be 0 or greater".into(),
        ));
    }

    let require_likes = get_bool(config, "require_likes");
    let min_likes = if require_likes {
        get_i64(config, "min_likes", 0)
    } else {
        0
    };
    if min_likes < 0 {
        return Err(AppError::BadRequest(
            "Minimum likes must be 0 or greater".into(),
        ));
    }

    let require_videos = get_bool(config, "require_videos");
    let min_videos = if require_videos {
        get_i64(config, "min_videos", 0)
    } else {
        0
    };
    if min_videos < 0 {
        return Err(AppError::BadRequest(
            "Minimum videos must be 0 or greater".into(),
        ));
    }

    let require_bio = get_bool(config, "require_bio");
    let min_bio_length = if require_bio {
        get_i64(config, "min_bio_length", 1).clamp(1, 80) as i32
    } else {
        0
    };

    Ok(TikTokConditions {
        require_verified,
        require_followers,
        min_followers,
        require_following,
        min_following,
        require_likes,
        min_likes,
        require_videos,
        min_videos,
        require_bio,
        min_bio_length,
    })
}
