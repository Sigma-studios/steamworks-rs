use macroquad::prelude::*;
use macroquad::prelude::KeyCode;
use macroquad::ui::*;
use std::sync::mpsc;
use steamworks::networking_types::{NetworkingIdentity, SendFlags};
use steamworks::*;

enum State {
    Menu,
    Lobby(LobbyState),
}

struct LobbyState {
    lobby_id: LobbyId,
    members: Vec<SteamId>,
    chat_log: Vec<String>,
    chat_input: String,
}

fn window_conf() -> Conf {
    Conf {
        window_title: "Lobby & Messages".to_string(),
        window_resizable: false,
        window_width: 900,
        window_height: 650,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let client = Client::init().unwrap();

    let matchmaking = client.matchmaking();
    let net_messages = client.networking_messages();
    let friends = client.friends();
    let my_id = client.user().steam_id();

    let mut state = State::Menu;

    // --- Channels for callbacks ---
    let (tx_lobby_created, rx_lobby_created) = mpsc::channel::<LobbyId>();
    let (tx_lobby_enter, rx_lobby_enter) = mpsc::channel::<LobbyEnter>();
    let (tx_chat_update, rx_chat_update) = mpsc::channel::<LobbyChatUpdate>();
    let (tx_chat_msg, rx_chat_msg) = mpsc::channel::<LobbyChatMsg>();

    // --- Register callbacks ---
    let _cb_lobby_enter = client.register_callback(move |ev: LobbyEnter| {
        let _ = tx_lobby_enter.send(ev);
    });

    let _cb_chat_update = client.register_callback(move |ev: LobbyChatUpdate| {
        let _ = tx_chat_update.send(ev);
    });

    let _cb_chat_msg = client.register_callback(move |ev: LobbyChatMsg| {
        let _ = tx_chat_msg.send(ev);
    });

    // Auto-accept incoming networking message sessions
    net_messages.session_request_callback(move |req| {
        println!("[Net] Accepting session request from {:?}", req.remote());
        req.accept();
    });

    net_messages.session_failed_callback(|info| {
        eprintln!("[Net] Session failed: {info:#?}");
    });

    loop {
        client.run_callbacks();
        clear_background(Color::from_rgba(30, 30, 30, 255));

        // --- Process callback events ---
        // Lobby created (from create_lobby async callback)
        if let Ok(lobby_id) = rx_lobby_created.try_recv() {
            println!("[Lobby] Created lobby {}", lobby_id.raw());
            let members = matchmaking.lobby_members(lobby_id);
            state = State::Lobby(LobbyState {
                lobby_id,
                members,
                chat_log: vec![format!("Lobby created: {}", lobby_id.raw())],
                chat_input: String::new(),
            });
        }

        // Lobby entered (for both host after creation and joiner)
        if let Ok(ev) = rx_lobby_enter.try_recv() {
            println!(
                "[Lobby] Entered lobby {} (response: {:?})",
                ev.lobby.raw(),
                ev.chat_room_enter_response
            );
            if ev.chat_room_enter_response == ChatRoomEnterResponse::Success {
                // If we're not already in this lobby (joiners land here)
                let already_in = matches!(&state, State::Lobby(ls) if ls.lobby_id == ev.lobby);
                if !already_in {
                    let members = matchmaking.lobby_members(ev.lobby);
                    state = State::Lobby(LobbyState {
                        lobby_id: ev.lobby,
                        members,
                        chat_log: vec![format!("Joined lobby: {}", ev.lobby.raw())],
                        chat_input: String::new(),
                    });
                }
            }
        }

        // Lobby chat update (player join/leave)
        while let Ok(ev) = rx_chat_update.try_recv() {
            if let State::Lobby(ref mut ls) = state {
                if ev.lobby == ls.lobby_id {
                    let name = friends.get_friend(ev.user_changed).name();
                    let action = match ev.member_state_change {
                        ChatMemberStateChange::Entered => "joined",
                        ChatMemberStateChange::Left => "left",
                        ChatMemberStateChange::Disconnected => "disconnected",
                        ChatMemberStateChange::Kicked => "was kicked",
                        ChatMemberStateChange::Banned => "was banned",
                    };
                    let msg = format!("** {} {} **", name, action);
                    println!("[Lobby] {}", msg);
                    ls.chat_log.push(msg);
                    ls.members = matchmaking.lobby_members(ls.lobby_id);
                }
            }
        }

        // Lobby chat messages
        while let Ok(ev) = rx_chat_msg.try_recv() {
            if let State::Lobby(ref mut ls) = state {
                if ev.lobby == ls.lobby_id {
                    let mut buf = vec![0u8; 4096];
                    let data = matchmaking.get_lobby_chat_entry(ev.lobby, ev.chat_id, &mut buf);
                    if let Ok(text) = std::str::from_utf8(data) {
                        let name = friends.get_friend(ev.user).name();
                        let msg = format!("[Lobby] {}: {}", name, text);
                        println!("{}", msg);
                        ls.chat_log.push(msg);
                    }
                }
            }
        }

        // Networking messages (P2P)
        if let State::Lobby(ref mut ls) = state {
            for message in net_messages.receive_messages_on_channel(0, 100) {
                let peer_identity = message.identity_peer();
                let data = message.data();
                if let Ok(text) = std::str::from_utf8(data) {
                    let name = if let Some(sid) = peer_identity.steam_id() {
                        friends.get_friend(sid).name()
                    } else {
                        "Unknown".to_string()
                    };
                    let msg = format!("[Net] {}: {}", name, text);
                    println!("{}", msg);
                    ls.chat_log.push(msg);
                }
            }
        }

        // --- Render ---
        let w = screen_width();
        let h = screen_height();

        match &mut state {
            State::Menu => {
                draw_text_ex(
                    "Press H to host",
                    w / 2.0 - 100.0,
                    h / 2.0 - 20.0,
                    TextParams {
                        font_size: 30,
                        color: WHITE,
                        ..Default::default()
                    },
                );
                draw_text_ex(
                    "Join lobbies through the steam friends list",
                    w / 2.0 - 250.0,
                    h / 2.0 + 20.0,
                    TextParams {
                        font_size: 20,
                        color: GRAY,
                        ..Default::default()
                    },
                );

                if is_key_pressed(KeyCode::H) {
                    let tx = tx_lobby_created.clone();
                    matchmaking.create_lobby(LobbyType::FriendsOnly, 4, move |result| {
                        if let Ok(lobby_id) = result {
                            let _ = tx.send(lobby_id);
                        }
                    });
                }
            }
            State::Lobby(ls) => {
                // --- Lobby ID ---
                draw_text_ex(
                    &format!("Lobby: {}", ls.lobby_id.raw()),
                    10.0,
                    25.0,
                    TextParams {
                        font_size: 20,
                        color: WHITE,
                        ..Default::default()
                    },
                );

                // --- Player list (left side) ---
                draw_text_ex(
                    "Players:",
                    10.0,
                    55.0,
                    TextParams {
                        font_size: 18,
                        color: YELLOW,
                        ..Default::default()
                    },
                );
                for (i, &member) in ls.members.iter().enumerate() {
                    let name = friends.get_friend(member).name();
                    let label = if member == my_id {
                        format!("{} (you)", name)
                    } else {
                        name
                    };
                    draw_text_ex(
                        &label,
                        15.0,
                        80.0 + 22.0 * i as f32,
                        TextParams {
                            font_size: 16,
                            color: WHITE,
                            ..Default::default()
                        },
                    );
                }

                // --- Chat log (right side) ---
                let chat_x = 220.0;
                let chat_y_start = 55.0;
                let chat_y_end = h - 60.0;
                let max_visible = ((chat_y_end - chat_y_start) / 18.0) as usize;
                let start = if ls.chat_log.len() > max_visible {
                    ls.chat_log.len() - max_visible
                } else {
                    0
                };
                for (i, msg) in ls.chat_log[start..].iter().enumerate() {
                    draw_text_ex(
                        msg,
                        chat_x,
                        chat_y_start + 18.0 * i as f32,
                        TextParams {
                            font_size: 16,
                            color: LIGHTGRAY,
                            ..Default::default()
                        },
                    );
                }

                // --- Chat input + buttons (bottom) ---
                let input_y = h - 35.0;
                widgets::Group::new(hash!(), vec2(300.0, 25.0))
                    .position(vec2(220.0, input_y))
                    .ui(&mut *root_ui(), |ui| {
                        ui.input_text(hash!(), "", &mut ls.chat_input);
                    });

                let mut send_lobby = false;
                let mut send_net = false;

                widgets::Group::new(hash!(), vec2(w - 220.0 - 10.0, 30.0))
                    .position(vec2(530.0, input_y))
                    .ui(&mut *root_ui(), |ui| {
                        send_lobby = ui.button(vec2(0.0, 0.0), "Send through lobby chat");
                        send_net = ui.button(vec2(190.0, 0.0), "Send through messages");
                    });

                if send_lobby && !ls.chat_input.is_empty() {
                    let text = ls.chat_input.clone();
                    println!("[Send Lobby] {}", text);
                    let _ = matchmaking.send_lobby_chat_message(ls.lobby_id, text.as_bytes());
                    ls.chat_input.clear();
                }

                if send_net && !ls.chat_input.is_empty() {
                    let text = ls.chat_input.clone();
                    println!("[Send Net] {}", text);
                    for &member in &ls.members {
                        if member != my_id {
                            let identity = NetworkingIdentity::new_steam_id(member);
                            let _ = net_messages.send_message_to_user(
                                identity,
                                SendFlags::RELIABLE,
                                text.as_bytes(),
                                0,
                            );
                        }
                    }
                    // Add our own message to the log locally
                    let my_name = friends.name();
                    ls.chat_log
                        .push(format!("[Net] {}: {}", my_name, text));
                    ls.chat_input.clear();
                }

                // --- Leave button ---
                let mut leave = false;
                widgets::Group::new(hash!(), vec2(80.0, 25.0))
                    .position(vec2(10.0, h - 35.0))
                    .ui(&mut *root_ui(), |ui| {
                        leave = ui.button(vec2(0.0, 0.0), "Leave");
                    });

                if leave {
                    println!("[Lobby] Leaving lobby {}", ls.lobby_id.raw());
                    matchmaking.leave_lobby(ls.lobby_id);
                    state = State::Menu;
                }
            }
        }

        next_frame().await;
    }
}
