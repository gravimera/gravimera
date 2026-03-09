pub mod ai;
pub mod backend;
pub mod cli;
pub mod chatterbox_onnx;
pub mod config;
pub mod effects;
pub mod language;
pub mod logging;
pub mod procedural;
pub mod render_plan;
pub mod sanitize;

use anyhow::{Context, Result, anyhow};
use rodio::buffer::SamplesBuffer;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

pub async fn run(cli: cli::Cli) -> Result<()> {
    let run_start = Instant::now();
    let settings = config::load_settings(&cli).with_context(|| {
        format!(
            "failed to load config from {}",
            settings_path_for_error(&cli)
        )
    })?;

    let availability = backend::BackendAvailability::detect(&settings);
    let log_path = logging::init()?;
    if cli.verbose {
        eprintln!("log: {}", log_path.to_string_lossy());
        if cli.no_ai {
            eprintln!("ai config: disabled (--no-ai)");
        } else {
            eprintln!(
                "ai config: base_url={} model={} effort={} wire_api={:?}",
                settings.base_url, settings.model, settings.model_reasoning_effort, settings.wire_api
            );
        }
    }

    match (&cli.command, cli.speak.is_empty()) {
        (Some(cli::Command::Doctor), true) => {
            run_doctor(&cli, &settings, availability, run_start, log_path).await
        }
        (Some(cli::Command::Speak(args)), true) => {
            run_single(&cli, &settings, availability, args, run_start, log_path).await
        }
        (None, false) => run_batch(&cli, &settings, availability, run_start, log_path).await,
        (Some(_), false) => Err(anyhow!(
            "cannot combine subcommands with `--speak`; use either `soundtest speak ...` or repeated `--speak` flags"
        )),
        (None, true) => Err(anyhow!(
            "no command provided. Use `soundtest speak <object> <text...>` or `soundtest --speak <object> <message>`"
        )),
    }
}

async fn run_doctor(
    cli: &cli::Cli,
    settings: &config::Settings,
    availability: backend::BackendAvailability,
    run_start: Instant,
    log_path: std::path::PathBuf,
) -> Result<()> {
    let platform = format!(
        "{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    println!("soundtest doctor");
    println!("platform: {platform}");
    if cli.verbose {
        println!("log: {}", log_path.to_string_lossy());
    }

    println!("backends:");

    match &availability.onnx_tts {
        Some(onnx) => {
            println!("  - onnx: ok");
            println!("    model_dir: {}", onnx.config.model_dir.to_string_lossy());
            println!("    voice: {}", onnx.voice_path.to_string_lossy());
            println!("    runtime: {}", onnx.runtime_path.to_string_lossy());
            println!(
                "    language_model: {}",
                onnx.language_model_path.to_string_lossy()
            );
        }
        None => {
            let configured = settings.onnx_model_dir.is_some();
            if configured {
                let reason = availability
                    .onnx_tts_error
                    .as_deref()
                    .unwrap_or("unknown error");
                println!("  - onnx: missing ({reason})");
            } else {
                println!("  - onnx: not configured (set onnx_model_dir / --onnx-model-dir)");
            }
        }
    }

    match &availability.system_tts {
        Some(system) => {
            println!("  - system: ok");
            println!("    binary: {}", system.binary.to_string_lossy());
        }
        None => println!("  - system: missing"),
    }

    println!("  - procedural: ok");

    if cli.verbose {
        println!("ai:");
        if cli.no_ai {
            println!("  - disabled (--no-ai)");
        } else {
            println!("  - enabled");
            println!("    base_url: {}", settings.base_url);
            println!("    model: {}", settings.model);
            println!("    reasoning_effort: {}", settings.model_reasoning_effort);
            println!(
                "    wire_api: {}",
                format!("{:?}", settings.wire_api).to_ascii_lowercase()
            );
        }
    }

    logging::info(
        "doctor.end",
        json!({
            "status": "ok",
            "duration_ms": run_start.elapsed().as_millis(),
        }),
    );

    Ok(())
}

async fn run_single(
    cli: &cli::Cli,
    settings: &config::Settings,
    availability: backend::BackendAvailability,
    args: &cli::SpeakArgs,
    run_start: Instant,
    log_path: std::path::PathBuf,
) -> Result<()> {
    let object = args.object.as_str();
    let message = args.text.join(" ");
    let volume = (cli.volume as f32) / 100.0;
    if message.trim().is_empty() {
        return Err(anyhow!(
            "text is required (e.g., `soundtest speak cat \"hello\"`)"
        ));
    }

    logging::info(
        "run.start",
        json!({
            "object": object,
            "requested_backend": format!("{:?}", cli.backend).to_ascii_lowercase(),
            "no_ai": cli.no_ai,
            "message_chars": message.chars().count(),
            "base_url": &settings.base_url,
            "model": &settings.model,
            "reasoning_effort": &settings.model_reasoning_effort,
            "wire_api": format!("{:?}", settings.wire_api).to_ascii_lowercase(),
            "available_backends": availability.available_backends_for_ai(),
            "dry_run": cli.dry_run,
            "verbose": cli.verbose,
            "volume": cli.volume,
            "meta": logging::event_fields(),
            "log_path": log_path.to_string_lossy(),
        }),
    );

    let mut render_plan = if cli.no_ai {
        build_no_ai_render_plan(object, &message, cli.backend, &availability)
    } else {
        let request = ai::RenderRequest {
            object,
            message: &message,
            requested_backend: cli.backend,
            availability,
        };

        let client = ai::OpenAiClient::new(&settings.base_url, &settings.token)?;

        let ai_result = client
            .generate_render_plan(settings, &request)
            .await
            .context("failed to generate render plan")?;

        if cli.verbose {
            eprintln!(
                "ai result: wire_api_used={} duration_ms={} output_chars={}",
                ai_result.wire_api_used, ai_result.duration_ms, ai_result.output_chars
            );
        }

        ai_result.plan
    };
    normalize_render_plan(&mut render_plan, &message);
    apply_effect_overrides(&mut render_plan, &EffectOverrides::from_cli(cli));

    if cli.verbose || cli.dry_run {
        eprintln!("--- render plan ---");
        eprintln!("{}", render_plan.to_debug_string());
        eprintln!("-------------------");
    }

    if cli.dry_run {
        match backend::preview_execution(settings, &render_plan, cli.backend) {
            Ok(preview) => {
                eprintln!("--- tools ---");
                eprintln!("{}", preview.format_tools());
                if let Some(text) = preview.text.as_deref() {
                    eprintln!("--- text ---");
                    eprintln!("{text}");
                }
                if let Some(proc) = preview.proc.as_deref() {
                    eprintln!("--- procedural ---");
                    eprintln!("{proc}");
                }
                eprintln!("--------------");
            }
            Err(err) => {
                eprintln!("tool preview error: {err:#}");
            }
        }
        logging::info(
            "run.end",
            json!({
                "status": "dry_run",
                "duration_ms": run_start.elapsed().as_millis(),
            }),
        );
        return Ok(());
    }

    let result =
        backend::execute_render_plan(settings, &render_plan, cli.backend, cli.verbose, volume).await;
    match &result {
        Ok(()) => logging::info(
            "run.end",
            json!({
                "status": "ok",
                "duration_ms": run_start.elapsed().as_millis(),
            }),
        ),
        Err(err) => logging::error(
            "run.end",
            json!({
                "status": "error",
                "error": format!("{err:#}"),
                "duration_ms": run_start.elapsed().as_millis(),
            }),
        ),
    }
    result
}

#[derive(Debug, Clone)]
struct BatchSpeakItem {
    object: String,
    message: String,
}

#[derive(Debug, Clone, Default)]
struct EffectOverrides {
    preset: Option<String>,
    amount: Option<f32>,
    speed: Option<f32>,
    pitch_semitones: Option<f32>,
    bass_db: Option<f32>,
    treble_db: Option<f32>,
    reverb: Option<f32>,
    distortion: Option<f32>,
}

impl EffectOverrides {
    fn from_cli(cli: &cli::Cli) -> Self {
        Self {
            preset: cli.preset.clone(),
            amount: cli.amount,
            speed: cli.speed,
            pitch_semitones: cli.pitch_semitones,
            bass_db: cli.bass_db,
            treble_db: cli.treble_db,
            reverb: cli.reverb,
            distortion: cli.distortion,
        }
    }

    fn any_set(&self) -> bool {
        self.preset.is_some()
            || self.amount.is_some()
            || self.speed.is_some()
            || self.pitch_semitones.is_some()
            || self.bass_db.is_some()
            || self.treble_db.is_some()
            || self.reverb.is_some()
            || self.distortion.is_some()
    }
}

#[derive(Debug)]
struct BatchRenderedItem {
    index: usize,
    object: String,
    output: String,
    plan_debug: String,
    audio: backend::RenderedAudio,
}

#[derive(Debug)]
struct BatchPlannedItem {
    index: usize,
    object: String,
    output: String,
    plan_debug: String,
    tools: String,
}

async fn run_batch(
    cli: &cli::Cli,
    settings: &config::Settings,
    availability: backend::BackendAvailability,
    run_start: Instant,
    log_path: std::path::PathBuf,
) -> Result<()> {
    let items = parse_batch_items(&cli.speak)?;
    let batch_count = items.len();
    let volume = (cli.volume as f32) / 100.0;

    if cli.ai_concurrency == 0 {
        return Err(anyhow!("--ai-concurrency must be >= 1"));
    }
    if cli.tts_concurrency == 0 {
        return Err(anyhow!("--tts-concurrency must be >= 1"));
    }
    if cli.dsp_concurrency == 0 {
        return Err(anyhow!("--dsp-concurrency must be >= 1"));
    }

    logging::info(
        "run.start",
        json!({
            "batch_count": items.len(),
            "requested_backend": format!("{:?}", cli.backend).to_ascii_lowercase(),
            "no_ai": cli.no_ai,
            "base_url": &settings.base_url,
            "model": &settings.model,
            "reasoning_effort": &settings.model_reasoning_effort,
            "wire_api": format!("{:?}", settings.wire_api).to_ascii_lowercase(),
            "available_backends": availability.available_backends_for_ai(),
            "dry_run": cli.dry_run,
            "verbose": cli.verbose,
            "volume": cli.volume,
            "ai_concurrency": cli.ai_concurrency,
            "tts_concurrency": cli.tts_concurrency,
            "dsp_concurrency": cli.dsp_concurrency,
            "meta": logging::event_fields(),
            "log_path": log_path.to_string_lossy(),
        }),
    );

    let client = if cli.no_ai {
        None
    } else {
        Some(ai::OpenAiClient::new(&settings.base_url, &settings.token)?)
    };

    let effect_overrides = EffectOverrides::from_cli(cli);
    let ai_sem = Arc::new(Semaphore::new(cli.ai_concurrency));
    let tts_sem = Arc::new(Semaphore::new(cli.tts_concurrency));
    let dsp_sem = Arc::new(Semaphore::new(cli.dsp_concurrency));
    let availability = Arc::new(availability);

    if cli.dry_run {
        let mut join_set: JoinSet<Result<BatchPlannedItem>> = JoinSet::new();
        for (index, item) in items.into_iter().enumerate() {
            let client = client.clone();
            let settings = settings.clone();
            let availability = availability.clone();
            let ai_sem = ai_sem.clone();
            let requested_backend = cli.backend;
            let verbose = cli.verbose;
            let no_ai = cli.no_ai;
            let effect_overrides = effect_overrides.clone();

            join_set.spawn(async move {
                let mut plan = if no_ai {
                    build_no_ai_render_plan(
                        &item.object,
                        &item.message,
                        requested_backend,
                        &*availability,
                    )
                } else {
                    let client = client.ok_or_else(|| anyhow!("internal error: missing AI client"))?;
                    let ai_result = {
                        let _permit = ai_sem
                            .acquire_owned()
                            .await
                            .map_err(|_| anyhow!("AI semaphore closed"))?;
                        let request = ai::RenderRequest {
                            object: &item.object,
                            message: &item.message,
                            requested_backend,
                            availability: (*availability).clone(),
                        };
                        client
                            .generate_render_plan(&settings, &request)
                            .await
                            .with_context(|| {
                                format!("failed to generate render plan for `{}`", item.object)
                            })?
                    };

                    ai_result.plan
                };
                normalize_render_plan(&mut plan, &item.message);
                apply_effect_overrides(&mut plan, &effect_overrides);

                let plan_debug = if verbose {
                    plan.to_debug_string()
                } else {
                    String::new()
                };

                let preview = backend::preview_execution(&settings, &plan, requested_backend);
                let tools = match preview {
                    Ok(p) => p.format_tools(),
                    Err(err) => format!("tool preview error: {err:#}"),
                };

                let output = match plan.backend {
                    render_plan::BackendKind::Procedural => plan.proc.clone().unwrap_or_default(),
                    render_plan::BackendKind::System | render_plan::BackendKind::Onnx => {
                        plan.text.clone().unwrap_or_default()
                    }
                };

                Ok(BatchPlannedItem {
                    index,
                    object: item.object,
                    output,
                    plan_debug,
                    tools,
                })
            });
        }

        let mut planned: Vec<Option<BatchPlannedItem>> = (0..batch_count).map(|_| None).collect();
        while let Some(result) = join_set.join_next().await {
            let item = result.map_err(|err| anyhow!("batch task join failed: {err}"))??;
            let index = item.index;
            if index >= planned.len() {
                return Err(anyhow!("internal error: batch index out of range"));
            }
            planned[index] = Some(item);
        }

        let mut planned: Vec<BatchPlannedItem> = planned
            .into_iter()
            .enumerate()
            .map(|(idx, item)| item.ok_or_else(|| anyhow!("missing batch result {idx}")))
            .collect::<Result<_>>()?;
        planned.sort_by_key(|i| i.index);

        for item in &planned {
            if cli.verbose {
                eprintln!("--- {} ---", item.object);
                eprintln!("{}", item.plan_debug);
                eprintln!("--- tools ---");
                eprintln!("{}", item.tools);
                if !item.output.trim().is_empty() {
                    eprintln!("--- output ---");
                    eprintln!("{}", item.output);
                }
                eprintln!("-------------");
            } else {
                eprintln!("{}: {}", item.object, item.output);
            }
        }

        logging::info(
            "run.end",
            json!({
                "status": "dry_run",
                "duration_ms": run_start.elapsed().as_millis(),
            }),
        );
        return Ok(());
    }

    let mut join_set: JoinSet<Result<BatchRenderedItem>> = JoinSet::new();

    for (index, item) in items.into_iter().enumerate() {
        let client = client.clone();
        let settings = settings.clone();
        let availability = availability.clone();
        let ai_sem = ai_sem.clone();
        let tts_sem = tts_sem.clone();
        let dsp_sem = dsp_sem.clone();
        let requested_backend = cli.backend;
        let verbose = cli.verbose;
        let no_ai = cli.no_ai;
        let effect_overrides = effect_overrides.clone();

        join_set.spawn(async move {
            let mut plan = if no_ai {
                build_no_ai_render_plan(
                    &item.object,
                    &item.message,
                    requested_backend,
                    &*availability,
                )
            } else {
                let client = client.ok_or_else(|| anyhow!("internal error: missing AI client"))?;
                let ai_result = {
                    let _permit = ai_sem
                        .acquire_owned()
                        .await
                        .map_err(|_| anyhow!("AI semaphore closed"))?;
                    let request = ai::RenderRequest {
                        object: &item.object,
                        message: &item.message,
                        requested_backend,
                        availability: (*availability).clone(),
                    };
                    client
                        .generate_render_plan(&settings, &request)
                        .await
                        .with_context(|| {
                            format!("failed to generate render plan for `{}`", item.object)
                        })?
                };

                ai_result.plan
            };
            normalize_render_plan(&mut plan, &item.message);
            apply_effect_overrides(&mut plan, &effect_overrides);
            let plan_debug = if verbose {
                plan.to_debug_string()
            } else {
                String::new()
            };

            let audio = match plan.backend {
                render_plan::BackendKind::Procedural => {
                    let tokens = plan
                        .proc
                        .as_deref()
                        .ok_or_else(|| anyhow!("render plan missing `proc:` for procedural backend"))?
                        .to_owned();
                    let samples = {
                        let _permit = dsp_sem
                            .acquire_owned()
                            .await
                            .map_err(|_| anyhow!("DSP semaphore closed"))?;
                        tokio::task::spawn_blocking(move || procedural::render_token_text(&tokens))
                            .await
                            .map_err(|err| anyhow!("procedural render task failed: {err}"))??
                    };
                    backend::RenderedAudio {
                        samples,
                        sample_rate: procedural::SAMPLE_RATE,
                    }
                }
                render_plan::BackendKind::System | render_plan::BackendKind::Onnx => {
                    let text = plan
                        .text
                        .as_deref()
                        .ok_or_else(|| anyhow!("render plan missing `text:` for TTS backend"))?
                        .to_owned();
                    let plan_backend = plan.backend;

                    let tts_audio = {
                        let _permit = tts_sem
                            .acquire_owned()
                            .await
                            .map_err(|_| anyhow!("TTS semaphore closed"))?;
                        let availability = (*availability).clone();
                        tokio::task::spawn_blocking(move || {
                            backend::synthesize_tts_mono_for_plan(
                                &availability,
                                requested_backend,
                                plan_backend,
                                &text,
                            )
                        })
                        .await
                        .map_err(|err| anyhow!("TTS task failed: {err}"))??
                    };

                    let effects_params = effects::EffectParams::from_spec(&plan.effects);
                    let sample_rate = tts_audio.sample_rate;
                    let processed = {
                        let _permit = dsp_sem
                            .acquire_owned()
                            .await
                            .map_err(|_| anyhow!("DSP semaphore closed"))?;
                        let samples = tts_audio.samples;
                        tokio::task::spawn_blocking(move || {
                            effects::apply_effects_mono(&samples, sample_rate, &effects_params)
                        })
                        .await
                        .map_err(|err| anyhow!("effects task failed: {err}"))?
                    };

                    backend::RenderedAudio {
                        samples: processed,
                        sample_rate,
                    }
                }
            };

            let output = match plan.backend {
                render_plan::BackendKind::Procedural => plan.proc.clone().unwrap_or_default(),
                render_plan::BackendKind::System | render_plan::BackendKind::Onnx => {
                    plan.text.clone().unwrap_or_default()
                }
            };

            Ok(BatchRenderedItem {
                index,
                object: item.object,
                output,
                plan_debug,
                audio,
            })
        });
    }

    let mut rendered: Vec<Option<BatchRenderedItem>> = (0..batch_count).map(|_| None).collect();
    while let Some(result) = join_set.join_next().await {
        let item = result.map_err(|err| anyhow!("batch task join failed: {err}"))??;
        let index = item.index;
        if index >= rendered.len() {
            return Err(anyhow!("internal error: batch index out of range"));
        }
        rendered[index] = Some(item);
    }

    let mut rendered: Vec<BatchRenderedItem> = rendered
        .into_iter()
        .enumerate()
        .map(|(idx, item)| item.ok_or_else(|| anyhow!("missing batch result {idx}")))
        .collect::<Result<_>>()?;
    rendered.sort_by_key(|i| i.index);

    if cli.verbose {
        for item in &rendered {
            eprintln!("--- {} ---", item.object);
            eprintln!("{}", item.plan_debug);
            eprintln!("-------------");
        }
    }

    for item in &rendered {
        println!("{}: {}", item.object, item.output);
    }

    let audios: Vec<backend::RenderedAudio> = rendered.into_iter().map(|i| i.audio).collect();
    let mixed = mix_to_single_track(audios, procedural::SAMPLE_RATE)?;

    let audio_out = backend::AudioOut::new(volume).context("failed to initialize audio output")?;
    let source = SamplesBuffer::new(1, mixed.sample_rate, mixed.samples);
    audio_out.play(source)?;

    logging::info(
        "run.end",
        json!({
            "status": "ok",
            "duration_ms": run_start.elapsed().as_millis(),
        }),
    );
    Ok(())
}

fn parse_batch_items(values: &[String]) -> Result<Vec<BatchSpeakItem>> {
    if values.is_empty() {
        return Err(anyhow!(
            "batch mode requires at least one `--speak <object> <message>` pair"
        ));
    }
    if values.len() % 2 != 0 {
        return Err(anyhow!("`--speak` expects exactly two values: <object> <message>"));
    }

    let mut items = Vec::with_capacity(values.len() / 2);
    for pair in values.chunks_exact(2) {
        let object = pair[0].trim().to_owned();
        let message = pair[1].trim().to_owned();
        if object.is_empty() {
            return Err(anyhow!("`--speak` object cannot be empty"));
        }
        if message.is_empty() {
            return Err(anyhow!("`--speak` message cannot be empty"));
        }
        items.push(BatchSpeakItem { object, message });
    }
    Ok(items)
}

fn normalize_render_plan(plan: &mut render_plan::RenderPlan, fallback_message: &str) {
    match plan.backend {
        render_plan::BackendKind::System | render_plan::BackendKind::Onnx => {
            if plan
                .text
                .as_deref()
                .map(|t| t.trim().is_empty())
                .unwrap_or(true)
            {
                plan.text = Some(fallback_message.trim().to_owned());
            }
        }
        render_plan::BackendKind::Procedural => {
            if plan
                .proc
                .as_deref()
                .map(|p| p.trim().is_empty())
                .unwrap_or(true)
            {
                plan.proc = Some("wind...".to_owned());
            }
        }
    }
}

fn apply_effect_overrides(plan: &mut render_plan::RenderPlan, overrides: &EffectOverrides) {
    if !overrides.any_set() {
        return;
    }
    if plan.backend == render_plan::BackendKind::Procedural {
        return;
    }

    if let Some(preset) = overrides.preset.as_deref() {
        let preset = preset.trim();
        if !preset.is_empty() {
            plan.effects.preset = preset.to_owned();
        }
    }
    if let Some(amount) = overrides.amount {
        plan.effects.amount = amount;
    }
    if let Some(speed) = overrides.speed {
        plan.effects.speed = Some(speed);
    }
    if let Some(pitch) = overrides.pitch_semitones {
        plan.effects.pitch_semitones = Some(pitch);
    }
    if let Some(bass_db) = overrides.bass_db {
        plan.effects.bass_db = Some(bass_db);
    }
    if let Some(treble_db) = overrides.treble_db {
        plan.effects.treble_db = Some(treble_db);
    }
    if let Some(reverb) = overrides.reverb {
        plan.effects.reverb = Some(reverb);
    }
    if let Some(distortion) = overrides.distortion {
        plan.effects.distortion = Some(distortion);
    }
}

fn build_no_ai_render_plan(
    _object: &str,
    message: &str,
    requested_backend: cli::BackendChoice,
    availability: &backend::BackendAvailability,
) -> render_plan::RenderPlan {
    let backend = match requested_backend {
        cli::BackendChoice::Auto => {
            if availability.onnx_tts.is_some() {
                render_plan::BackendKind::Onnx
            } else if availability.system_tts.is_some() {
                render_plan::BackendKind::System
            } else {
                render_plan::BackendKind::Procedural
            }
        }
        cli::BackendChoice::Onnx => render_plan::BackendKind::Onnx,
        cli::BackendChoice::System => render_plan::BackendKind::System,
        cli::BackendChoice::Procedural => render_plan::BackendKind::Procedural,
    };

    let message = message.trim();
    let effects = effects::EffectSpec::default();

    match backend {
        render_plan::BackendKind::Procedural => render_plan::RenderPlan {
            backend,
            text: None,
            proc: Some(message.to_owned()),
            effects,
            raw: format!("backend: procedural\nproc: {message}"),
        },
        render_plan::BackendKind::System => render_plan::RenderPlan {
            backend,
            text: Some(message.to_owned()),
            proc: None,
            effects,
            raw: format!("backend: system\ntext: {message}"),
        },
        render_plan::BackendKind::Onnx => render_plan::RenderPlan {
            backend,
            text: Some(message.to_owned()),
            proc: None,
            effects,
            raw: format!("backend: onnx\ntext: {message}"),
        },
    }
}

fn mix_to_single_track(
    mut audios: Vec<backend::RenderedAudio>,
    target_sample_rate: u32,
) -> Result<backend::RenderedAudio> {
    if audios.is_empty() {
        return Err(anyhow!("no sounds to mix"));
    }

    for audio in &mut audios {
        if audio.sample_rate != target_sample_rate {
            let factor = audio.sample_rate as f32 / target_sample_rate as f32;
            audio.samples = resample_linear_mono(&audio.samples, factor);
            audio.sample_rate = target_sample_rate;
        }
    }

    let max_len = audios
        .iter()
        .map(|a| a.samples.len())
        .max()
        .unwrap_or_default();
    if max_len == 0 {
        return Err(anyhow!("all rendered audio buffers were empty"));
    }

    let voices = audios.len().max(1) as f32;
    let gain = if voices <= 1.0 { 1.0 } else { 1.0 / voices.sqrt() };

    let mut mixed = vec![0.0f32; max_len];
    for audio in audios {
        for (i, &s) in audio.samples.iter().enumerate() {
            mixed[i] += s * gain;
        }
    }

    apply_limiter(&mut mixed, 0.98);
    Ok(backend::RenderedAudio {
        samples: mixed,
        sample_rate: target_sample_rate,
    })
}

fn resample_linear_mono(input: &[f32], factor: f32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    if factor <= 0.0 || (factor - 1.0).abs() < 1e-6 {
        return input.to_owned();
    }

    let out_len = ((input.len() as f32) / factor).ceil().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f32 * factor;
        let idx = pos.floor() as usize;
        let frac = pos - (idx as f32);
        let s0 = input.get(idx).copied().unwrap_or(0.0);
        let s1 = input.get(idx + 1).copied().unwrap_or(s0);
        out.push(s0 * (1.0 - frac) + s1 * frac);
    }
    out
}

fn apply_limiter(samples: &mut [f32], ceiling: f32) {
    let ceiling = ceiling.max(0.1).min(1.0);
    let mut max_abs = 0.0f32;
    for &s in samples.iter() {
        max_abs = max_abs.max(s.abs());
    }
    if max_abs > ceiling {
        let gain = ceiling / max_abs;
        for s in samples {
            *s *= gain;
        }
    }
}

fn settings_path_for_error(cli: &cli::Cli) -> String {
    cli.config
        .as_deref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| config::default_config_path().to_string_lossy().to_string())
}
