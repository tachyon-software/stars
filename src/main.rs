use dotenv::dotenv;
use serenity::{
    client::Client,
    model::{
        channel::{Message, Reaction, ReactionType},
        gateway::Ready,
        id::{ChannelId, EmojiId, MessageId},
    },
    prelude::{Context, EventHandler},
};

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use chrono::prelude::*;
use chrono::{offset::Utc, DateTime};

#[derive(Debug)]
struct WatchedMessage {
    star_count: usize,
    message: Message,
}

impl WatchedMessage {
    fn on_star_added(&mut self) {
        self.star_count += 1;
    }

    fn is_ready_for_pinning(&self) -> bool {
        self.star_count >= 10
    }

    fn url(&self) -> String {
        format!(
            "https://discordapp.com/channels/{}/{}",
            self.message.channel_id, self.message.id
        )
    }

    fn new(
        context: &Context,
        reaction: &Reaction,
        kind: &ReactionKind,
    ) -> Result<WatchedMessage, String> {
        Ok(WatchedMessage {
            star_count: match kind {
                ReactionKind::AdminStar => 10,
                ReactionKind::UserStar => 0,
            },
            message: reaction
                .message(&context.http)
                .map_err(|err| format!("Could not retrieve message: {}", err))?,
        })
    }
}

struct Handler {
    watched_messages: Arc<RwLock<HashMap<MessageId, WatchedMessage>>>,
    admin_star_id: EmojiId,
    star_id: EmojiId,
    instantiation_time: Instant,
    starboard_channel: ChannelId,
}

enum ReactionKind {
    AdminStar,
    UserStar,
}

impl Handler {
    fn new(admin_star_id: u64, star_id: u64, starboard_channel: u64) -> Handler {
        let watched_messages = Arc::new(RwLock::new(HashMap::with_capacity(32)));
        let admin_star_id = admin_star_id.into();
        let star_id = star_id.into();
        let starboard_channel = starboard_channel.into();
        let instantiation_time = Instant::now();

        Handler {
            watched_messages,
            admin_star_id,
            star_id,
            instantiation_time,
            starboard_channel,
        }
    }

    fn add_message_to_starboard(
        &self,
        ctx: &Context,
        watched_message: &WatchedMessage,
    ) -> Result<(), String> {
        let star_time: DateTime<Utc> = Utc::now();
        let author = watched_message
            .message
            .author_nick(ctx)
            .unwrap_or_else(|| watched_message.message.author.name.clone());
        self.starboard_channel
            .send_message(&ctx.http, |m| {
                m.embed(|e| {
                    let has_message = !&watched_message.message.content.is_empty();
                    if has_message {
                        e.description(&watched_message.message.content);
                    }
                    let attachments = &watched_message.message.attachments;
                    if attachments.len() == 1 {
                        if has_message {
                            e.thumbnail(attachments.get(0).unwrap().url.clone());
                        } else {
                            e.image(attachments.get(0).unwrap().url.clone());
                        }
                    } else if attachments.len() > 1 {
                        let mut attachments_str = attachments.iter().fold(
                            String::with_capacity(77 * attachments.len()),
                            |mut acc, a| {
                                acc.push_str(&*a.url);
                                acc.push('\n');
                                acc
                            },
                        );
                        attachments_str.pop();
                        e.description(attachments_str);
                    }
                    e.color(0xFFCC36);
                    e.author(|a| {
                        a.name(author);
                        a.url(watched_message.url());
                        a.icon_url(watched_message.message.author.face());
                        a
                    });
                    e.timestamp(&star_time);
                    e
                })
            })
            .map_err(|err| format!("Could not send message to star board: {}", err))
            .map(|_| ())
    }

    fn is_valid_reaction(&self, reaction: &Reaction) -> Option<ReactionKind> {
        if let ReactionType::Custom {
            id,
            animated: _,
            name: _,
        } = reaction.emoji
        {
            if id == self.admin_star_id {
                return Some(ReactionKind::AdminStar);
            } else if id == self.star_id {
                return Some(ReactionKind::UserStar);
            }
        }
        None
    }
}

impl EventHandler for Handler {
    fn reaction_add(&self, context: Context, reaction: Reaction) {
        dbg!(&reaction);
        let reaction_kind = self.is_valid_reaction(&reaction);
        if reaction_kind.is_none() {
            return;
        }
        let reaction_kind = reaction_kind.unwrap();
        if let Ok(mut write_lock) = self.watched_messages.write() {
            if let Some(ref mut watched_message) = write_lock.get_mut(&reaction.message_id) {
                watched_message.on_star_added();
            } else {
                match WatchedMessage::new(&context, &reaction, &reaction_kind) {
                    Ok(message) => {
                        write_lock.insert(reaction.message_id, message);
                    }
                    Err(err) => eprintln!("Error creating WatchedMessage: {}", err),
                }
            }
        }
        let mut to_delete = None;
        if let Ok(read_lock) = self.watched_messages.read() {
            if let Some(watched_message) = read_lock.get(&reaction.message_id) {
                if watched_message.is_ready_for_pinning() {
                    match self.add_message_to_starboard(&context, watched_message) {
                        Ok(_) => to_delete = Some(watched_message.message.id),
                        Err(err) => match reaction
                            .channel_id
                            .send_message(&context.http, |m| m.content(err))
                        {
                            Ok(_) => {}
                            Err(err) => eprintln!("Error reporting error: {}", err),
                        },
                    }
                }
            }
        }
        if let Some(msg_id) = to_delete {
            if let Ok(mut write_lock) = self.watched_messages.write() {
                write_lock.remove(&msg_id);
            }
        }
    }

    fn ready(&self, _context: Context, _data_about_bot: Ready) {
        println!(
            "Bot ready after {}ms",
            Instant::now()
                .duration_since(self.instantiation_time)
                .as_millis()
        );
    }
}

fn main() -> Result<(), String> {
    dotenv().map_err(|e| format!("Error loading dotenv: {}", e))?;

    // Login with a bot token from the environment
    let mut client = Client::new(
        &std::env::var("DISCORD_TOKEN")
            .map_err(|err| format!("Error getting discord token: {}", err))?,
        Handler::new(
            std::env::var("ADMIN_STAR_EMOJI_ID")
                .map_err(|err| format!("Error getting admin emoji star id: {}", err))?
                .parse::<u64>()
                .map_err(|err| format!("Error parsing admin emoji star id as u64: {}", err))?,
            std::env::var("STAR_EMOJI_ID")
                .map_err(|err| format!("Error getting emoji star id: {}", err))?
                .parse::<u64>()
                .map_err(|err| format!("Error parsing emoji star id as u64: {}", err))?,
            std::env::var("STARBOARD_CHANNEL_ID")
                .map_err(|err| format!("Error getting starboard channel id: {}", err))?
                .parse::<u64>()
                .map_err(|err| format!("Error parsing starbound channel id as u64: {}", err))?,
        ),
    )
    .map_err(|err| format!("Error instantiating client: {}", err))?;

    // start listening for events by starting a single shard
    client
        .start()
        .map_err(|err| format!("Error starting server: {}", err))
}
