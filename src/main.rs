// src/main.rs
mod commands;
mod session_store;
mod utils;

use serenity::all::*;
use serenity::async_trait;
use std::sync::Arc;
use session_store::SessionStore;

pub const MSP_EMOJI: &str = "<a:msp:1534303316065124392>";

struct Handler {
    store: Arc<SessionStore>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("Bot connecté en tant que {}", ready.user.name);

        ctx.set_activity(Some(ActivityData::playing("moviestarplanet2.com")));

        let commands = vec![
            commands::login::register(),
            commands::logout::register(),
            commands::gender::register(),
            commands::mood::register(),
            commands::quests::register(),
            commands::quelle_humeur::register(),
            commands::set_status::register(),
            commands::accept_friends::register(),
            commands::read_messages::register(),
        ];

        Command::set_global_commands(&ctx.http, commands)
            .await
            .expect("Impossible d'enregistrer les commandes globales");

        tracing::info!("Slash commands enregistrées !");
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(cmd) => {
                self.handle_command(ctx, cmd).await;
            }
            Interaction::Component(comp) => {
                self.handle_component(ctx, comp).await;
            }
            _ => {}
        }
    }
}

impl Handler {
    async fn handle_command(&self, ctx: Context, cmd: CommandInteraction) {
        match cmd.data.name.as_str() {
            "connexion"            => commands::login::run(&ctx, &cmd, Arc::clone(&self.store)).await,
            "deconnexion"          => commands::logout::run(&ctx, &cmd, Arc::clone(&self.store)).await,
            "sexe"                 => commands::gender::run(&ctx, &cmd, Arc::clone(&self.store)).await,
            "humeur"               => commands::mood::run(&ctx, &cmd, Arc::clone(&self.store)).await,
            "compléter-quêtes"     => commands::quests::run(&ctx, &cmd, Arc::clone(&self.store)).await,
            "quelle-humeur"        => commands::quelle_humeur::run(&ctx, &cmd, Arc::clone(&self.store)).await,
            "statut"               => commands::set_status::run(&ctx, &cmd, Arc::clone(&self.store)).await,
            "accepter-demandes-amis" => commands::accept_friends::run(&ctx, &cmd, Arc::clone(&self.store)).await,
            "messages" => commands::read_messages::run(&ctx, &cmd, Arc::clone(&self.store)).await,
            other => tracing::warn!("Commande inconnue : {other}"),
        }
    }

    async fn handle_component(&self, ctx: Context, comp: ComponentInteraction) {
        match comp.data.custom_id.as_str() {
            id if id.starts_with("mood_select") => {
                commands::mood::handle_select(&ctx, &comp, Arc::clone(&self.store)).await;
            }
            id if id.starts_with("copy_mood:") => {
                commands::quelle_humeur::handle_copy_mood(&ctx, &comp, Arc::clone(&self.store)).await;
            }
            id if id.starts_with("login_copy_mood:") => {
                commands::login::handle_copy_mood(&ctx, &comp, Arc::clone(&self.store)).await;
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(false)
        .init();

    let token = std::env::var("DISCORD_TOKEN")
        .expect("DISCORD_TOKEN manquant dans l'environnement");

    let store = Arc::new(SessionStore::new());

    let intents = GatewayIntents::empty();

    let mut client = Client::builder(&token, intents)
        .event_handler(Handler {
            store: Arc::clone(&store),
        })
        .await?;

    tracing::info!("Démarrage du bot...");
    client.start().await?;

    Ok(())
}