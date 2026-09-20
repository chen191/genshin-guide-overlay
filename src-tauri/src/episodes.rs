use crate::settings::GuideTarget;
use serde_json::Value;
use std::time::Duration;
use url::Url;

const VIEW_API: &str = "https://api.bilibili.com/x/web-interface/view";

#[derive(Debug, Clone, PartialEq, Eq)]
enum VideoId {
    Bvid(String),
    Aid(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EpisodeCursor {
    video_id: VideoId,
    page: u32,
    cid: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct NextEpisode {
    pub source_url: Url,
    pub message: String,
}

pub type PreviousEpisode = NextEpisode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EpisodeDirection {
    Previous,
    Next,
}

pub async fn fetch_next_episode(target: &GuideTarget) -> Result<NextEpisode, String> {
    fetch_adjacent_episode(target, EpisodeDirection::Next).await
}

pub async fn fetch_previous_episode(target: &GuideTarget) -> Result<PreviousEpisode, String> {
    fetch_adjacent_episode(target, EpisodeDirection::Previous).await
}

async fn fetch_adjacent_episode(
    target: &GuideTarget,
    direction: EpisodeDirection,
) -> Result<NextEpisode, String> {
    let cursor = EpisodeCursor::from_target(target)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| format!("初始化 B 站分集连接失败：{error}"))?;
    let request = client
        .get(VIEW_API)
        .header(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131 Safari/537.36",
        )
        .header(reqwest::header::REFERER, "https://www.bilibili.com/");
    let request = match &cursor.video_id {
        VideoId::Bvid(value) => request.query(&[("bvid", value)]),
        VideoId::Aid(value) => request.query(&[("aid", value.to_string())]),
    };
    let response = request
        .send()
        .await
        .map_err(|error| format!("连接 B 站分集信息失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "B 站分集信息暂时不可用（HTTP {}）",
            response.status().as_u16()
        ));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|error| format!("读取 B 站分集信息失败：{error}"))?;
    adjacent_episode_from_payload(&cursor, &payload, direction)
}

impl EpisodeCursor {
    fn from_target(target: &GuideTarget) -> Result<Self, String> {
        let url = &target.content_url;
        if url.host_str() != Some("player.bilibili.com") || url.path() != "/player.html" {
            return Err("当前页面不是可切换分集的 B 站播放器".to_string());
        }
        let mut bvid = None;
        let mut aid = None;
        let mut page = None;
        let mut fallback_page = None;
        let mut cid = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "bvid" if valid_bvid(&value) => bvid = Some(value.into_owned()),
                "aid" => aid = value.parse::<u64>().ok().filter(|value| *value > 0),
                "page" => page = value.parse::<u32>().ok().filter(|value| *value > 0),
                "p" => fallback_page = value.parse::<u32>().ok().filter(|value| *value > 0),
                "cid" => cid = value.parse::<u64>().ok().filter(|value| *value > 0),
                _ => {}
            }
        }
        let video_id = bvid
            .map(VideoId::Bvid)
            .or_else(|| aid.map(VideoId::Aid))
            .ok_or_else(|| "当前 B 站播放器缺少有效的视频编号".to_string())?;
        Ok(Self {
            video_id,
            page: page.or(fallback_page).unwrap_or(1),
            cid,
        })
    }
}

#[cfg(test)]
fn next_episode_from_payload(
    cursor: &EpisodeCursor,
    payload: &Value,
) -> Result<NextEpisode, String> {
    adjacent_episode_from_payload(cursor, payload, EpisodeDirection::Next)
}

#[cfg(test)]
fn previous_episode_from_payload(
    cursor: &EpisodeCursor,
    payload: &Value,
) -> Result<PreviousEpisode, String> {
    adjacent_episode_from_payload(cursor, payload, EpisodeDirection::Previous)
}

fn adjacent_episode_from_payload(
    cursor: &EpisodeCursor,
    payload: &Value,
    direction: EpisodeDirection,
) -> Result<NextEpisode, String> {
    if payload.get("code").and_then(Value::as_i64) != Some(0) {
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .unwrap_or("请稍后重试");
        return Err(format!("B 站分集查询失败：{message}"));
    }
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "B 站没有返回有效的分集信息".to_string())?;
    let canonical_bvid = data
        .get("bvid")
        .and_then(Value::as_str)
        .filter(|value| valid_bvid(value));
    let canonical_aid = data
        .get("aid")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0);
    let canonical_id = canonical_bvid
        .map(|value| VideoId::Bvid(value.to_string()))
        .or_else(|| canonical_aid.map(VideoId::Aid))
        .ok_or_else(|| "B 站分集信息缺少有效的视频编号".to_string())?;

    let pages = data
        .get("pages")
        .and_then(Value::as_array)
        .ok_or_else(|| "B 站没有返回当前视频的分集列表".to_string())?;
    let current_index = if let Some(cid) = cursor.cid {
        pages
            .iter()
            .position(|page| page.get("cid").and_then(Value::as_u64) == Some(cid))
    } else {
        pages
            .iter()
            .position(|page| page_number(page) == Some(cursor.page))
    }
    .ok_or_else(|| "当前分集不在 B 站返回的分集列表中".to_string())?;
    let adjacent_page = match direction {
        EpisodeDirection::Previous => current_index
            .checked_sub(1)
            .and_then(|index| pages.get(index)),
        EpisodeDirection::Next => pages.get(current_index + 1),
    };
    if let Some(adjacent_page) = adjacent_page {
        let page = page_number(adjacent_page)
            .ok_or_else(|| format!("{}缺少有效的分集编号", direction.label()))?;
        return Ok(NextEpisode {
            source_url: source_url(&canonical_id, page, direction)?,
            message: format!("已打开第 {page} 集"),
        });
    }

    let episodes = data
        .get("ugc_season")
        .and_then(|season| season.get("sections"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|section| section.get("episodes").and_then(Value::as_array))
        .flatten()
        .collect::<Vec<_>>();
    if let Some(current_episode_index) = episodes
        .iter()
        .position(|episode| episode_matches(episode, canonical_bvid, canonical_aid))
    {
        let adjacent_episode = match direction {
            EpisodeDirection::Previous => episodes[..current_episode_index]
                .iter()
                .rev()
                .copied()
                .find_map(|episode| collection_candidate(episode, &canonical_id)),
            EpisodeDirection::Next => episodes[current_episode_index + 1..]
                .iter()
                .copied()
                .find_map(|episode| collection_candidate(episode, &canonical_id)),
        };
        if let Some((episode, next_id)) = adjacent_episode {
            let page = collection_page(episode, direction)?;
            return Ok(NextEpisode {
                source_url: source_url(&next_id, page, direction)?,
                message: format!("已打开合集{}", direction.label()),
            });
        }
    }

    Err(format!(
        "当前视频没有{}（仅支持同视频分集或 B 站合集）",
        direction.label()
    ))
}

impl EpisodeDirection {
    fn label(self) -> &'static str {
        match self {
            Self::Previous => "上一集",
            Self::Next => "下一集",
        }
    }
}

fn collection_page(episode: &Value, direction: EpisodeDirection) -> Result<u32, String> {
    let pages = episode.get("pages").and_then(Value::as_array);
    match direction {
        EpisodeDirection::Previous => pages
            .and_then(|pages| pages.iter().rev().find_map(page_number))
            .ok_or_else(|| "合集上一集缺少有效的分集列表".to_string()),
        EpisodeDirection::Next => Ok(pages
            .and_then(|pages| pages.first())
            .and_then(page_number)
            .unwrap_or(1)),
    }
}

fn collection_candidate<'a>(
    episode: &'a Value,
    canonical_id: &VideoId,
) -> Option<(&'a Value, VideoId)> {
    let video_id = episode_video_id(episode)?;
    (video_id != *canonical_id).then_some((episode, video_id))
}

fn page_number(page: &Value) -> Option<u32> {
    page.get("page")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn episode_matches(episode: &Value, bvid: Option<&str>, aid: Option<u64>) -> bool {
    bvid.is_some_and(|expected| episode.get("bvid").and_then(Value::as_str) == Some(expected))
        || aid.is_some_and(|expected| episode.get("aid").and_then(Value::as_u64) == Some(expected))
}

fn episode_video_id(episode: &Value) -> Option<VideoId> {
    episode
        .get("bvid")
        .and_then(Value::as_str)
        .filter(|value| valid_bvid(value))
        .map(|value| VideoId::Bvid(value.to_string()))
        .or_else(|| {
            episode
                .get("aid")
                .and_then(Value::as_u64)
                .filter(|value| *value > 0)
                .map(VideoId::Aid)
        })
}

fn source_url(video_id: &VideoId, page: u32, direction: EpisodeDirection) -> Result<Url, String> {
    let path = match video_id {
        VideoId::Bvid(value) if valid_bvid(value) => value.clone(),
        VideoId::Aid(value) if *value > 0 => format!("av{value}"),
        _ => return Err(format!("{}缺少有效的视频编号", direction.label())),
    };
    let mut url = Url::parse("https://www.bilibili.com/video")
        .map_err(|error| format!("无法创建{}地址：{error}", direction.label()))?;
    url.path_segments_mut()
        .map_err(|_| format!("无法创建{}地址", direction.label()))?
        .push(&path);
    if page > 1 {
        url.query_pairs_mut().append_pair("p", &page.to_string());
    }
    Ok(url)
}

fn valid_bvid(value: &str) -> bool {
    value.len() == 12
        && value
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("BV1"))
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::resolve_guide_target;
    use serde_json::json;

    fn cursor(page: u32) -> EpisodeCursor {
        EpisodeCursor {
            video_id: VideoId::Bvid("BV1vK4y1P74F".to_string()),
            page,
            cid: None,
        }
    }

    #[test]
    fn reads_the_current_video_and_page_from_the_pure_player() {
        let target = resolve_guide_target(
            "https://player.bilibili.com/player.html?aid=170001&bvid=BV1oPDUYwEWD&page=3",
        )
        .unwrap();
        assert_eq!(
            EpisodeCursor::from_target(&target).unwrap(),
            EpisodeCursor {
                video_id: VideoId::Bvid("BV1oPDUYwEWD".to_string()),
                page: 3,
                cid: None,
            }
        );

        let aid_target =
            resolve_guide_target("https://player.bilibili.com/player.html?aid=170001&page=0")
                .unwrap();
        assert_eq!(
            EpisodeCursor::from_target(&aid_target).unwrap(),
            EpisodeCursor {
                video_id: VideoId::Aid(170001),
                page: 1,
                cid: None,
            }
        );
    }

    #[test]
    fn accepts_p_alias_and_uses_cid_as_the_stronger_page_identity() {
        let p_target =
            resolve_guide_target("https://player.bilibili.com/player.html?bvid=BV1oPDUYwEWD&p=2")
                .unwrap();
        assert_eq!(EpisodeCursor::from_target(&p_target).unwrap().page, 2);

        let cid_target = resolve_guide_target(
            "https://player.bilibili.com/player.html?bvid=BV1oPDUYwEWD&page=1&cid=300",
        )
        .unwrap();
        let cid_cursor = EpisodeCursor::from_target(&cid_target).unwrap();
        let payload = json!({
            "code": 0,
            "data": {
                "bvid": "BV1oPDUYwEWD",
                "aid": 123,
                "pages": [
                    {"page": 1, "cid": 100},
                    {"page": 2, "cid": 300},
                    {"page": 3, "cid": 500}
                ]
            }
        });
        let next = next_episode_from_payload(&cid_cursor, &payload).unwrap();
        assert_eq!(
            next.source_url.as_str(),
            "https://www.bilibili.com/video/BV1oPDUYwEWD?p=3"
        );

        let previous = previous_episode_from_payload(
            &EpisodeCursor {
                video_id: VideoId::Bvid("BV1oPDUYwEWD".to_string()),
                page: 1,
                cid: Some(500),
            },
            &payload,
        )
        .unwrap();
        assert_eq!(
            previous.source_url.as_str(),
            "https://www.bilibili.com/video/BV1oPDUYwEWD?p=2"
        );
    }

    #[test]
    fn chooses_the_next_page_from_the_same_video_in_api_order() {
        let payload = json!({
            "code": 0,
            "data": {
                "bvid": "BV1vK4y1P74F",
                "aid": 123,
                "pages": [
                    {"page": 1, "part": "第一集"},
                    {"page": 3, "part": "第三集"}
                ],
                "related": [{"bvid": "BV1oPDUYwEWD"}]
            }
        });

        let next = next_episode_from_payload(&cursor(1), &payload).unwrap();
        assert_eq!(
            next.source_url.as_str(),
            "https://www.bilibili.com/video/BV1vK4y1P74F?p=3"
        );
        assert_eq!(next.message, "已打开第 3 集");
    }

    #[test]
    fn chooses_the_previous_page_from_the_same_video_in_sparse_api_order() {
        let payload = json!({
            "code": 0,
            "data": {
                "bvid": "BV1vK4y1P74F",
                "aid": 123,
                "pages": [
                    {"page": 1, "part": "第一集"},
                    {"page": 3, "part": "第三集"},
                    {"page": 8, "part": "第八集"}
                ]
            }
        });

        let previous = previous_episode_from_payload(&cursor(8), &payload).unwrap();
        assert_eq!(
            previous.source_url.as_str(),
            "https://www.bilibili.com/video/BV1vK4y1P74F?p=3"
        );
        assert_eq!(previous.message, "已打开第 3 集");
    }

    #[test]
    fn finishes_all_pages_before_crossing_to_the_next_collection_video() {
        let payload = json!({
            "code": 0,
            "data": {
                "bvid": "BV1vK4y1P74F",
                "aid": 123,
                "pages": [{"page": 1}, {"page": 2}],
                "ugc_season": {
                    "sections": [
                        {"episodes": [{"bvid": "BV1vK4y1P74F", "aid": 123}]},
                        {"episodes": [{
                            "bvid": "BV1oPDUYwEWD",
                            "aid": 456,
                            "pages": [{"page": 2}]
                        }]}
                    ]
                },
                "related": [{"bvid": "BV1unrelated0"}]
            }
        });

        let same_video = next_episode_from_payload(&cursor(1), &payload).unwrap();
        assert_eq!(
            same_video.source_url.as_str(),
            "https://www.bilibili.com/video/BV1vK4y1P74F?p=2"
        );

        let next_video = next_episode_from_payload(&cursor(2), &payload).unwrap();
        assert_eq!(
            next_video.source_url.as_str(),
            "https://www.bilibili.com/video/BV1oPDUYwEWD?p=2"
        );
        assert_eq!(next_video.message, "已打开合集下一集");
    }

    #[test]
    fn crosses_sections_to_the_previous_aid_video_and_uses_its_last_valid_page() {
        let aid_cursor = EpisodeCursor {
            video_id: VideoId::Aid(456),
            page: 1,
            cid: None,
        };
        let payload = json!({
            "code": 0,
            "data": {
                "aid": 456,
                "pages": [{"page": 1}],
                "ugc_season": {
                    "sections": [
                        {"episodes": [{
                            "aid": 123,
                            "pages": [
                                {"page": 2},
                                {"page": 5},
                                {"page": 0},
                                {"page": "invalid"}
                            ]
                        }]},
                        {"episodes": [{"aid": 456, "pages": [{"page": 1}]}]}
                    ]
                },
                "related": [{"bvid": "BV1oPDUYwEWD"}]
            }
        });

        let previous = previous_episode_from_payload(&aid_cursor, &payload).unwrap();
        assert_eq!(
            previous.source_url.as_str(),
            "https://www.bilibili.com/video/av123?p=5"
        );
        assert_eq!(previous.message, "已打开合集上一集");
    }

    #[test]
    fn previous_collection_video_requires_at_least_one_valid_page() {
        let without_pages = json!({
            "code": 0,
            "data": {
                "bvid": "BV1vK4y1P74F",
                "aid": 123,
                "pages": [{"page": 1}],
                "ugc_season": {"sections": [{"episodes": [
                    {"bvid": "BV1oPDUYwEWD", "aid": 456},
                    {"bvid": "BV1vK4y1P74F", "aid": 123}
                ]}]}
            }
        });
        assert!(previous_episode_from_payload(&cursor(1), &without_pages)
            .unwrap_err()
            .contains("缺少有效的分集列表"));

        let invalid_pages = json!({
            "code": 0,
            "data": {
                "bvid": "BV1vK4y1P74F",
                "aid": 123,
                "pages": [{"page": 1}],
                "ugc_season": {"sections": [{"episodes": [
                    {
                        "bvid": "BV1oPDUYwEWD",
                        "aid": 456,
                        "pages": [{"page": 0}, {"page": "invalid"}]
                    },
                    {"bvid": "BV1vK4y1P74F", "aid": 123}
                ]}]}
            }
        });
        assert!(previous_episode_from_payload(&cursor(1), &invalid_pages)
            .unwrap_err()
            .contains("缺少有效的分集列表"));
    }

    #[test]
    fn keeps_the_next_collection_page_one_fallback_for_compatibility() {
        let payload = json!({
            "code": 0,
            "data": {
                "bvid": "BV1vK4y1P74F",
                "aid": 123,
                "pages": [{"page": 1}],
                "ugc_season": {"sections": [{"episodes": [
                    {"bvid": "BV1vK4y1P74F", "aid": 123},
                    {"bvid": "BV1oPDUYwEWD", "aid": 456}
                ]}]}
            }
        });

        let next = next_episode_from_payload(&cursor(1), &payload).unwrap();
        assert_eq!(
            next.source_url.as_str(),
            "https://www.bilibili.com/video/BV1oPDUYwEWD"
        );
    }

    #[test]
    fn never_falls_back_to_related_or_recommended_videos() {
        let payload = json!({
            "code": 0,
            "data": {
                "bvid": "BV1vK4y1P74F",
                "aid": 123,
                "pages": [{"page": 1}],
                "related": [{"bvid": "BV1oPDUYwEWD"}]
            }
        });

        let error = next_episode_from_payload(&cursor(1), &payload).unwrap_err();
        assert!(error.contains("没有下一集"));

        let error = previous_episode_from_payload(&cursor(1), &payload).unwrap_err();
        assert!(error.contains("没有上一集"));
    }

    #[test]
    fn reports_invalid_api_and_missing_current_page_responses() {
        let rejected = json!({"code": -404, "message": "啥都木有"});
        assert!(next_episode_from_payload(&cursor(1), &rejected)
            .unwrap_err()
            .contains("啥都木有"));

        let missing_page = json!({
            "code": 0,
            "data": {
                "bvid": "BV1vK4y1P74F",
                "aid": 123,
                "pages": [{"page": 2}]
            }
        });
        assert!(next_episode_from_payload(&cursor(1), &missing_page)
            .unwrap_err()
            .contains("当前分集"));
    }
}
