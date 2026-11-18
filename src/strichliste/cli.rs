use std::io::{self, BufRead, IsTerminal, Write};

use eyre::{Result, ensure, eyre};
use serde_json::{Value, json};

use super::{Client, Options, Request, User};
use crate::cli::{GlobalArgs, StrichlisteCommand};

pub async fn run(global: &GlobalArgs, command: StrichlisteCommand) -> Result<()> {
    let (request, comment) = match command {
        StrichlisteCommand::Transactions { limit, offset } => {
            if global.dry_run {
                return print(global, json!({"dry_run": true}));
            }
            let page = Client::new(Options::from_global(global))?
                .transactions(limit as usize, offset)
                .await?;
            let transactions = page.entries;
            if global.json {
                let transactions: Vec<_> = transactions
                    .iter()
                    .map(|transaction| {
                        json!({
                            "id": transaction.id,
                            "created_at": transaction.created_at,
                            "value_eur": transaction.value_eur,
                            "comment": transaction.comment,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    json!({
                        "transactions": transactions,
                        "offset": page.offset,
                        "overall_count": page.total,
                        "next_offset": page.next_offset,
                    })
                );
            } else if !global.quiet {
                if transactions.is_empty() {
                    println!("No recent transactions.");
                }
                for transaction in transactions {
                    println!(
                        "{}  {:+.2} EUR  {}  #{}  {}",
                        transaction.created_at.escape_debug(),
                        transaction.value_eur,
                        transaction.kind(),
                        transaction.id,
                        transaction.comment.as_deref().unwrap_or("").escape_debug(),
                    );
                }
            }
            return Ok(());
        }
        StrichlisteCommand::Balance => (Request::Balance, String::new()),
        StrichlisteCommand::Rank => (Request::Rank, String::new()),
        StrichlisteCommand::Pay { amount, comment } => (Request::Pay(amount), comment),
        StrichlisteCommand::Add { amount, comment } => (Request::Add(amount), comment),
        StrichlisteCommand::Send { amount, comment } => {
            let value = -super::validate_amount(amount)?;
            if global.dry_run {
                return print(
                    global,
                    json!({
                        "dry_run": true, "submitted": false, "value_eur": value,
                        "recipient": "interactive selection", "comment": comment,
                    }),
                );
            }
            ensure!(
                io::stdin().is_terminal() && io::stderr().is_terminal(),
                "send requires an interactive terminal for recipient search"
            );
            let client = Client::new(Options::from_global(global))?;
            let current = client.current_user().await?;
            let users = client.users().await?;
            let Some(recipient) = search(
                &users,
                current.id,
                &mut io::stdin().lock(),
                &mut io::stderr().lock(),
            )?
            else {
                return Ok(());
            };
            return print(global, client.transfer(recipient, amount, &comment).await?);
        }
    };
    let result = if global.dry_run {
        let mut preview = request.preview()?;
        if request.value()?.is_some() {
            preview["comment"] = json!(comment);
        }
        preview
    } else {
        Client::new(Options::from_global(global))?
            .execute(&request, &comment)
            .await?
    };
    print(global, result)
}

fn prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
) -> Result<Option<String>> {
    write!(output, "{label}")?;
    output.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 || line.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(line.trim().to_owned()))
}

fn search<'a>(
    users: &'a [User],
    own_id: u64,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Option<&'a User>> {
    loop {
        let Some(query) = prompt(input, output, "Recipient name (empty to cancel): ")? else {
            return Ok(None);
        };
        let matches: Vec<_> = users
            .iter()
            .filter(|user| {
                user.id != own_id && user.name.to_lowercase().contains(&query.to_lowercase())
            })
            .collect();
        if matches.is_empty() {
            writeln!(output, "No matching users.")?;
            continue;
        }
        if matches.len() > 20 {
            writeln!(
                output,
                "{} matches; please narrow the search.",
                matches.len()
            )?;
            continue;
        }
        for (index, user) in matches.iter().enumerate() {
            writeln!(
                output,
                "{}: {} (id {})",
                index + 1,
                user.name.escape_debug(),
                user.id
            )?;
        }
        let Some(selection) = prompt(
            input,
            output,
            "Recipient number (0 to search again; empty to cancel): ",
        )?
        else {
            return Ok(None);
        };
        if selection == "0" {
            continue;
        }
        if let Ok(index) = selection.parse::<usize>() {
            if let Some(user) = index.checked_sub(1).and_then(|index| matches.get(index)) {
                return Ok(Some(user));
            }
        }
        writeln!(output, "Invalid selection; search again.")?;
    }
}

fn print(global: &GlobalArgs, value: Value) -> Result<()> {
    if global.json {
        println!("{value}");
    } else if !global.quiet {
        if value["dry_run"] == true {
            println!("dry-run: {value}");
        } else if let Some(balance) = value["balance_eur"].as_f64() {
            println!("Balance: {balance} €");
        } else if let Some(rank) = value["rank"].as_u64() {
            println!(
                "Rank: {rank} (weighted purchases: {})",
                value["weighted_count_of_purchases"]
            );
        } else if value["submitted"] == true {
            let euros = value["value_eur"]
                .as_f64()
                .ok_or_else(|| eyre!("missing transaction value"))?;
            println!("Booked: {euros} €");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn search_selects_exact_recipient_and_excludes_self() {
        let users = [
            User::parse(&json!({"id": 27, "name": "Alice"})).unwrap(),
            User::parse(&json!({"id": 30, "name": "Alice Other"})).unwrap(),
            User::parse(&json!({"id": 31, "name": "Alice Another"})).unwrap(),
        ];
        let mut output = Vec::new();
        let selected = search(&users, 27, &mut Cursor::new("ALICE\n2\n"), &mut output)
            .unwrap()
            .unwrap();
        assert_eq!(selected.id, 31);
        let rendered = String::from_utf8(output).unwrap();
        assert!(!rendered.contains("(id 27)"));
        assert!(rendered.contains("1: Alice Other"));
        assert!(rendered.contains("2: Alice Another"));

        for input in [
            "",
            "\n",
            "Alice\n",
            "Alice\n\n",
            "Alice\n0\n",
            "missing\n",
            "Alice\n99\n",
        ] {
            assert!(
                search(&users, 27, &mut Cursor::new(input), &mut Vec::new())
                    .unwrap()
                    .is_none()
            );
        }
        let selected = search(
            &users,
            27,
            &mut Cursor::new("missing\nAlice\n99\nAnother\n1\n"),
            &mut Vec::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.id, 31);
    }
}
