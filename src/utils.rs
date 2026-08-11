// src/utils.rs
use serenity::all::*;
use crate::MSP_EMOJI;

pub async fn error_embed(
    ctx:     &Context,
    cmd:     &CommandInteraction,
    title:   &str,
    desc:    &str,
) {
    let embed = CreateEmbed::new()
        .title(format!("❌  {}", title))
        .description(desc)
        .color(0xED4245)
        .footer(CreateEmbedFooter::new("MovieStarPlanet2 Bot • Created by Just a Banana!"))
        .timestamp(Timestamp::now());

    let _ = cmd
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content("")
                .embed(embed),
        )
        .await;
}

pub async fn defer_ephemeral(ctx: &Context, cmd: &CommandInteraction) {
    let _ = cmd.defer_ephemeral(&ctx.http).await;

    let _ = cmd
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content(format!("{} **Traitement en cours...**", MSP_EMOJI)),
        )
        .await;
}

pub async fn defer_ephemeral_login(ctx: &Context, cmd: &CommandInteraction) {
    let _ = cmd.defer_ephemeral(&ctx.http).await;

    let _ = cmd
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content(format!("{} **Connexion en cours...**", MSP_EMOJI)),
        )
        .await;
}

pub async fn defer_ephemeral_quests(ctx: &Context, cmd: &CommandInteraction) {
    let _ = cmd.defer_ephemeral(&ctx.http).await;

    let _ = cmd
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content(format!("{} **Cela peut prendre du temps...**", MSP_EMOJI)),
        )
        .await;
}

pub async fn defer_component_ephemeral(ctx: &Context, comp: &ComponentInteraction) {
    let _ = comp.defer_ephemeral(&ctx.http).await;

    let _ = comp
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content(format!("{} **Traitement en cours...**", MSP_EMOJI)),
        )
        .await;
}