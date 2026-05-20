use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TrainJob {
    pub id: i64,
    pub base_model: String,
    pub method: String,
    pub dataset_size: i64,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub output_model: Option<String>,
    pub benchmark_delta: Option<f64>,
}

/// Recommendations for base models to train.
pub fn suggest_base_model(use_case: &crate::config::schema::TrainingUse) -> &'static str {
    match use_case {
        crate::config::schema::TrainingUse::Coding => "deepseek-ai/deepseek-coder-7b-instruct-v1.5",
        crate::config::schema::TrainingUse::Writing => "mistralai/Mistral-7B-Instruct-v0.3",
        crate::config::schema::TrainingUse::General => "meta-llama/Meta-Llama-3.1-8B-Instruct",
    }
}

/// Probe for available training environments.
pub fn detect_environment(pref: &crate::config::schema::TrainingEnvironment) -> TrainingEnvironment {
    let has_docker = std::process::Command::new("docker")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let has_python = std::process::Command::new("python")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    match pref {
        crate::config::schema::TrainingEnvironment::Docker if has_docker => TrainingEnvironment::Docker,
        crate::config::schema::TrainingEnvironment::Venv if has_python => TrainingEnvironment::Venv,
        _ => {
            if has_docker {
                TrainingEnvironment::Docker
            } else if has_python {
                TrainingEnvironment::Venv
            } else {
                TrainingEnvironment::None
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum TrainingEnvironment {
    Docker,
    Venv,
    None,
}

#[derive(Debug)]
pub enum TrainerKind {
    Unsloth,
    Axolotl,
}

/// Probe whether Unsloth or Axolotl is importable as a Python module.
pub fn detect_trainer() -> Option<TrainerKind> {
    let probe = |module: &str| -> bool {
        std::process::Command::new("python")
            .args(["-c", &format!("import {module}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if probe("unsloth") {
        return Some(TrainerKind::Unsloth);
    }
    if probe("axolotl") {
        return Some(TrainerKind::Axolotl);
    }
    None
}

pub fn insert_job(conn: &Connection, base_model: &str, method: &str, dataset_size: i64) -> Result<i64> {
    let ts = Utc::now().timestamp();
    conn.execute(
        "INSERT INTO train_jobs (started_at, base_model, method, dataset_size, status) \
         VALUES (?1, ?2, ?3, ?4, 'running')",
        params![ts, base_model, method, dataset_size],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn finish_job(conn: &Connection, job_id: i64, status: &str, output_model: Option<&str>) -> Result<()> {
    let ts = Utc::now().timestamp();
    conn.execute(
        "UPDATE train_jobs SET status = ?1, finished_at = ?2, output_model = ?3 WHERE id = ?4",
        params![status, ts, output_model, job_id],
    )?;
    Ok(())
}

pub fn list_jobs(conn: &Connection) -> Result<Vec<TrainJob>> {
    let mut stmt = conn.prepare(
        "SELECT id, base_model, COALESCE(method, 'lora'), COALESCE(dataset_size, 0), \
         status, started_at, finished_at, output_model, benchmark_delta \
         FROM train_jobs ORDER BY started_at DESC LIMIT 20",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(TrainJob {
            id: row.get(0)?,
            base_model: row.get(1)?,
            method: row.get(2)?,
            dataset_size: row.get(3)?,
            status: row.get(4)?,
            started_at: row.get(5)?,
            finished_at: row.get(6)?,
            output_model: row.get(7)?,
            benchmark_delta: row.get(8)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Register a model (trained or imported) into model_registry.
pub fn register_model(
    conn: &Connection,
    name: &str,
    provider: &str,
    model_id: &str,
    path: Option<&str>,
) -> Result<()> {
    let ts = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO model_registry (name, provider, model_id, path, imported_at, is_active) \
         VALUES (?1, ?2, ?3, ?4, ?5, 1)",
        params![name, provider, model_id, path, ts],
    )?;
    Ok(())
}

/// Write a Python training script to a temp file and return its path.
fn write_training_script(
    trainer: &TrainerKind,
    base_model: &str,
    data_path: &Path,
    output_dir: &Path,
    method: &str,
) -> Result<PathBuf> {
    let script_path = std::env::temp_dir().join("zedplus_train.py");

    // Forward-slash paths work in Python on Windows
    let data_str = data_path.to_string_lossy().replace('\\', "/");
    let out_str = output_dir.to_string_lossy().replace('\\', "/");

    let script = match trainer {
        TrainerKind::Unsloth => format!(
            r#"import sys
from unsloth import FastLanguageModel
from datasets import load_dataset
from trl import SFTTrainer
from transformers import TrainingArguments

print("Loading model: {base_model}", flush=True)
model, tokenizer = FastLanguageModel.from_pretrained(
    model_name="{base_model}",
    max_seq_length=2048,
    load_in_4bit=True,
)

if "{method}" == "lora":
    print("Applying LoRA adapters...", flush=True)
    model = FastLanguageModel.get_peft_model(
        model,
        r=16,
        target_modules=["q_proj", "v_proj", "k_proj", "o_proj"],
        lora_alpha=16,
        lora_dropout=0.0,
        bias="none",
        use_gradient_checkpointing=True,
    )

print("Loading dataset: {data_str}", flush=True)
dataset = load_dataset("json", data_files="{data_str}", split="train")
print(f"Dataset size: {{len(dataset)}} examples", flush=True)

trainer = SFTTrainer(
    model=model,
    tokenizer=tokenizer,
    train_dataset=dataset,
    dataset_text_field="instruction",
    max_seq_length=2048,
    args=TrainingArguments(
        per_device_train_batch_size=2,
        gradient_accumulation_steps=4,
        warmup_steps=10,
        num_train_epochs=1,
        learning_rate=2e-4,
        fp16=True,
        logging_steps=1,
        output_dir="{out_str}",
        report_to="none",
    ),
)

print("Training started...", flush=True)
trainer.train()

print("Saving model to {out_str}", flush=True)
model.save_pretrained("{out_str}")
tokenizer.save_pretrained("{out_str}")
print("ZEDPLUS_TRAIN_SUCCESS", flush=True)
"#
        ),

        TrainerKind::Axolotl => {
            let yaml_path = std::env::temp_dir().join("zedplus_axolotl.yml");
            let yaml = format!(
                "base_model: {base_model}\n\
                 model_type: AutoModelForCausalLM\n\
                 tokenizer_type: AutoTokenizer\n\
                 load_in_4bit: true\n\
                 datasets:\n\
                 \x20 - path: {data_str}\n\
                 \x20   type: alpaca\n\
                 adapter: {adapter}\n\
                 lora_r: 16\n\
                 lora_alpha: 32\n\
                 lora_dropout: 0.05\n\
                 lora_target_modules:\n\
                 \x20 - q_proj\n\
                 \x20 - v_proj\n\
                 output_dir: {out_str}\n\
                 sequence_len: 2048\n\
                 micro_batch_size: 2\n\
                 gradient_accumulation_steps: 4\n\
                 num_epochs: 1\n\
                 learning_rate: 0.0002\n",
                adapter = if method == "full" { "null" } else { "lora" },
            );
            std::fs::write(&yaml_path, yaml)?;
            let yaml_str = yaml_path.to_string_lossy().replace('\\', "/");
            format!(
                "import subprocess, sys\n\
                 result = subprocess.run(\n\
                 \x20   [sys.executable, '-m', 'axolotl.cli.train', '{yaml_str}'],\n\
                 \x20   check=False)\n\
                 if result.returncode == 0:\n\
                 \x20   print('ZEDPLUS_TRAIN_SUCCESS', flush=True)\n\
                 else:\n\
                 \x20   sys.exit(result.returncode)\n"
            )
        }
    };

    std::fs::write(&script_path, script)?;
    Ok(script_path)
}

/// Spawn the training process, tail its output, and update the train_jobs row on completion.
pub async fn run_training(
    job_id: i64,
    base_model: &str,
    data_path: &Path,
    method: &str,
    output_dir: &Path,
    db_path: &Path,
) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let trainer = detect_trainer();
    let output_dir_str = output_dir.to_string_lossy().to_string();

    let Some(trainer_kind) = trainer else {
        println!("No training framework found (Unsloth or Axolotl not installed).");
        println!();
        println!("To install Unsloth:  pip install unsloth");
        println!("To install Axolotl:  pip install axolotl");
        println!();
        println!("Export your data with: zedplus distill --weighted --out training.jsonl");
        println!("Data file: {}", data_path.display());
        let conn = crate::db::open(db_path)?;
        finish_job(&conn, job_id, "failed", None)?;
        return Ok(());
    };

    let script_path = write_training_script(&trainer_kind, base_model, data_path, output_dir, method)?;

    println!("Starting training job #{job_id}");
    println!("  Base model : {base_model}");
    println!("  Method     : {method}");
    println!("  Data       : {}", data_path.display());
    println!("  Output dir : {}", output_dir.display());
    println!();

    let python = if cfg!(target_os = "windows") { "python" } else { "python3" };

    let mut child = tokio::process::Command::new(python)
        .arg(&script_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to launch Python training process. Is Python installed?")?;

    let stdout = child.stdout.take().expect("captured stdout");
    let stderr = child.stderr.take().expect("captured stderr");

    let mut out_lines = BufReader::new(stdout).lines();
    let mut err_lines = BufReader::new(stderr).lines();

    let mut success_signal = false;
    let mut stderr_done = false;

    loop {
        tokio::select! {
            line = out_lines.next_line() => {
                match line? {
                    Some(l) => {
                        if l.contains("ZEDPLUS_TRAIN_SUCCESS") {
                            success_signal = true;
                        } else {
                            println!("{l}");
                        }
                    }
                    None => break,
                }
            }
            line = err_lines.next_line(), if !stderr_done => {
                match line {
                    Ok(Some(l)) => eprintln!("{l}"),
                    _ => stderr_done = true,
                }
            }
        }
    }

    // Drain remaining stderr
    while let Ok(Some(l)) = err_lines.next_line().await {
        eprintln!("{l}");
    }

    let exit_status = child.wait().await?;
    let succeeded = success_signal && exit_status.success();

    let conn = crate::db::open(db_path)?;
    if succeeded {
        finish_job(&conn, job_id, "complete", Some(&output_dir_str))?;
        println!();
        println!("Training complete!");
        println!("Output: {output_dir_str}");
        println!();
        println!("Register the model with:");
        println!("  zedplus model import \"{output_dir_str}\" --name my-lora");
    } else {
        finish_job(&conn, job_id, "failed", None)?;
        eprintln!();
        eprintln!("Training failed (exit code: {:?}).", exit_status.code());
    }

    Ok(())
}

#[derive(Debug)]
pub struct AutoTrainSuggestion {
    pub new_conversations: i64,
    pub base_model: String,
    pub last_trained_at: i64,
    pub reason: String,
}

/// Return an auto-train suggestion if thresholds are met.
pub fn check_auto_train(
    conn: &Connection, 
    cfg: &crate::config::schema::TrainingConfig
) -> Result<Option<AutoTrainSuggestion>> {
    let last_trained_at: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(finished_at), 0) FROM train_jobs WHERE status = 'complete'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // 1. Check Volume Threshold
    let new_conversations: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM usage WHERE ts > ?1",
            params![last_trained_at],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if new_conversations >= cfg.auto_train_min_new as i64 {
        return Ok(Some(AutoTrainSuggestion {
            new_conversations,
            base_model: "auto".to_string(), // will be resolved later
            last_trained_at,
            reason: format!("Volume threshold reached ({} new messages)", new_conversations),
        }));
    }

    // 2. Check "Significant Session" Heuristics
    // A session is significant if it resulted in high cost or many file writes since last training.
    let sig: (f64, i64) = conn.query_row(
        "SELECT SUM(cost_usd), COUNT(*) FROM usage WHERE ts > ?1 AND (cost_usd > ?2 OR task_type = 'code_review')",
        params![last_trained_at, cfg.significance_thresholds.min_cost_usd],
        |row| Ok((row.get(0).unwrap_or(0.0), row.get(1)?))
    ).unwrap_or((0.0, 0));

    if sig.0 > cfg.significance_thresholds.min_cost_usd {
        return Ok(Some(AutoTrainSuggestion {
            new_conversations,
            base_model: "auto".to_string(),
            last_trained_at,
            reason: format!("High-value session detected (${:.2} cost)", sig.0),
        }));
    }

    Ok(None)
}
