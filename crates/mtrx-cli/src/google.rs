//! `mtrx google`: the Google account used for Meet links.

use std::path::Path;

use anyhow::{Context, Result};
use clap::Subcommand;
use mtrx_core::SecretStore;
use mtrx_google::{Google, GoogleClient};

#[derive(Subcommand)]
pub enum GoogleCommand {
    /// Sign in with your own Google OAuth client (type "Desktop app").
    Login {
        /// OAuth client ID from Google Cloud.
        #[arg(long)]
        client_id: String,
        /// Read the client secret from the first line of stdin instead of prompting.
        #[arg(long)]
        client_secret_stdin: bool,
    },
    /// Show the signed-in Google account.
    Status,
    /// Sign out and forget the Google tokens.
    Logout,
    /// Print a new Google Meet link.
    Meet,
}

pub async fn run(command: GoogleCommand, data_root: &Path, secrets: SecretStore) -> Result<()> {
    let google = Google::new(data_root, secrets);
    match command {
        GoogleCommand::Login {
            client_id,
            client_secret_stdin,
        } => {
            let client_secret = crate::read_secret(client_secret_stdin, "Client secret: ")?;
            let client = GoogleClient {
                client_id,
                client_secret,
            };
            let email = google
                .sign_in(client, |url| {
                    println!("Opening the Google sign-in page in your browser:\n{url}");
                    let _ = open::that_detached(url.as_str());
                })
                .await
                .context("Google sign-in failed")?;
            println!("Signed in to Google as {email}");
        }
        GoogleCommand::Status => match google.account()? {
            Some(email) => println!("Signed in to Google as {email}"),
            None => println!("Not signed in to Google"),
        },
        GoogleCommand::Logout => {
            google.sign_out().await?;
            println!("Signed out of Google");
        }
        GoogleCommand::Meet => println!("{}", google.create_meeting().await?),
    }
    Ok(())
}
