use minicode_history::{append_runtime_message, runtime_messages};
use minicode_prompt::build_system_prompt;
use minicode_tool::get_tool_registry;
use minicode_types::{AgentStep, ChatMessage, ModelAdapter};
use serde_json::Value;

pub trait AgentTurnCallbacks: Send {
    /// 工具开始执行时触发。
    fn on_tool_start(&mut self, _tool_name: &str, _input: &Value) {}
    /// 工具返回结果时触发。
    fn on_tool_result(&mut self, _tool_name: &str, _output: &str, _is_error: bool) {}
    /// 助手给出最终消息时触发。
    fn on_assistant_message(&mut self, _content: &str) {}
    /// 助手给出进度消息时触发。
    fn on_progress_message(&mut self, _content: &str) {}
}

/// 判断助手回复是否为空白内容。
fn is_empty_assistant_response(content: &str) -> bool {
    content.trim().is_empty()
}

/// 拼接 stop reason 与 block 信息用于诊断输出。
fn format_diagnostics(
    stop_reason: Option<&str>,
    block_types: Option<&[String]>,
    ignored: Option<&[String]>,
) -> String {
    let mut parts = vec![];
    if let Some(s) = stop_reason
        && !s.is_empty()
    {
        parts.push(format!("stop_reason={s}"));
    }
    if let Some(b) = block_types
        && !b.is_empty()
    {
        parts.push(format!("blocks={}", b.join(",")));
    }
    if let Some(i) = ignored
        && !i.is_empty()
    {
        parts.push(format!("ignored={}", i.join(",")));
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!(" 诊断信息: {}。", parts.join("; "))
    }
}

/// 执行一轮 agent 对话，循环处理助手输出与工具调用。
pub async fn run_agent_turn(
    model: &dyn ModelAdapter,
    max_steps: Option<usize>,
    mut callbacks: Option<&mut (dyn AgentTurnCallbacks + Send)>,
) {
    let mut empty_retry = 0usize;
    let mut recover_retry = 0usize;
    let mut tool_error_count = 0usize;
    let mut saw_tool_result = false;

    let limit = max_steps.unwrap_or(64);

    for _ in 0..limit {
        let next = match model.next(&get_messages_with_system()).await {
            Ok(step) => step,
            Err(err) => {
                if let Some(cb) = callbacks.as_deref_mut() {
                    cb.on_assistant_message(&format!("请求失败: {err}"));
                }
                append_runtime_message(ChatMessage::Assistant {
                    content: format!("请求失败: {err}"),
                });
                return;
            }
        };

        match next {
            AgentStep::Assistant {
                content,
                kind,
                diagnostics,
            } => {
                let is_empty = is_empty_assistant_response(&content);

                if !is_empty && kind.as_deref() == Some("progress") {
                    if let Some(cb) = callbacks.as_deref_mut() {
                        cb.on_progress_message(&content);
                    }
                    append_runtime_message(ChatMessage::AssistantProgress {
                        content: content.clone(),
                    });
                    append_runtime_message(ChatMessage::Minicode {
                        content: "继续，紧接着上一条进度消息执行。请给出下一步具体工具调用、代码修改，或在任务确实完成时给出最终答案。".to_string(),
                    });
                    continue;
                }

                if is_empty {
                    let stop_reason = diagnostics.as_ref().and_then(|d| d.stop_reason.as_deref());
                    let ignored = diagnostics
                        .as_ref()
                        .and_then(|d| d.ignored_block_types.clone())
                        .unwrap_or_default();
                    let is_recover = (stop_reason == Some("pause_turn")
                        || stop_reason == Some("max_tokens"))
                        && ignored.iter().any(|x| x == "thinking");

                    if is_recover && recover_retry < 3 {
                        recover_retry += 1;
                        let progress = if stop_reason == Some("max_tokens") {
                            "模型在 thinking 阶段触发 max_tokens，正在继续请求后续步骤..."
                                .to_string()
                        } else {
                            "模型返回 pause_turn，正在继续请求后续步骤...".to_string()
                        };
                        if let Some(cb) = callbacks.as_deref_mut() {
                            cb.on_progress_message(&progress);
                        }
                        append_runtime_message(ChatMessage::AssistantProgress {
                            content: progress,
                        });
                        append_runtime_message(ChatMessage::Minicode {
                            content: "继续，从你刚才中断的位置直接执行下一步，给出具体工具调用或代码修改。".to_string(),
                        });
                        continue;
                    }

                    if empty_retry < 2 {
                        empty_retry += 1;
                        let retry_prompt = if saw_tool_result {
                            "上一条回复为空，且你刚收到工具结果。请立即继续下一步，先根据工具报错修正参数或改用可行方案，再执行。"
                        } else {
                            "上一条回复为空。请立即继续，给出下一步具体工具调用或代码修改。"
                        };
                        append_runtime_message(ChatMessage::Minicode {
                            content: retry_prompt.to_string(),
                        });
                        continue;
                    }

                    let diag = format_diagnostics(
                        diagnostics.as_ref().and_then(|d| d.stop_reason.as_deref()),
                        diagnostics.as_ref().and_then(|d| d.block_types.as_deref()),
                        diagnostics
                            .as_ref()
                            .and_then(|d| d.ignored_block_types.as_deref()),
                    );
                    let fallback = if saw_tool_result {
                        if tool_error_count > 0 {
                            format!(
                                "工具执行后模型返回空响应，已停止当前回合。最近有 {tool_error_count} 个工具报错；请重试或调整方案。{diag}"
                            )
                        } else {
                            format!(
                                "工具执行后模型返回空响应，已停止当前回合。请重试或要求模型继续。{diag}"
                            )
                        }
                    } else {
                        format!("模型返回空响应，已停止当前回合。请重试。{diag}")
                    };
                    if let Some(cb) = callbacks.as_deref_mut() {
                        cb.on_assistant_message(&fallback);
                    }
                    append_runtime_message(ChatMessage::Assistant { content: fallback });
                    return;
                }

                if let Some(cb) = callbacks.as_deref_mut() {
                    cb.on_assistant_message(&content);
                }
                append_runtime_message(ChatMessage::Assistant { content });
                return;
            }
            AgentStep::ToolCalls {
                calls,
                content,
                content_kind,
                ..
            } => {
                let content_only_final =
                    content.is_some() && content_kind.as_deref() != Some("progress");
                if let Some(c) = content {
                    if content_kind.as_deref() == Some("progress") {
                        if let Some(cb) = callbacks.as_deref_mut() {
                            cb.on_progress_message(&c);
                        }
                        append_runtime_message(ChatMessage::AssistantProgress { content: c });
                        append_runtime_message(ChatMessage::Minicode {
                            content: "继续，给出下一步工具调用或最终答案。".to_string(),
                        });
                    } else {
                        if let Some(cb) = callbacks.as_deref_mut() {
                            cb.on_assistant_message(&c);
                        }
                        append_runtime_message(ChatMessage::Assistant { content: c });
                    }
                }

                if calls.is_empty() {
                    if content_only_final {
                        return;
                    }
                    continue;
                }

                for call in calls {
                    if let Some(cb) = callbacks.as_deref_mut() {
                        cb.on_tool_start(&call.tool_name, &call.input);
                    }
                    let result = get_tool_registry()
                        .execute(&call.tool_name, call.input.clone())
                        .await;
                    if let Some(cb) = callbacks.as_deref_mut() {
                        cb.on_tool_result(&call.tool_name, &result.output, !result.ok);
                    }
                    saw_tool_result = true;
                    if !result.ok {
                        tool_error_count += 1;
                    }

                    append_runtime_message(ChatMessage::AssistantToolCall {
                        tool_use_id: call.id.clone(),
                        tool_name: call.tool_name.clone(),
                        input: call.input,
                    });
                    append_runtime_message(ChatMessage::ToolResult {
                        tool_use_id: call.id,
                        tool_name: call.tool_name,
                        content: result.output.clone(),
                        is_error: !result.ok,
                    });

                    if result.await_user {
                        let question = result.output.trim();
                        if !question.is_empty() {
                            if let Some(cb) = callbacks.as_deref_mut() {
                                cb.on_assistant_message(question);
                            }
                            append_runtime_message(ChatMessage::Assistant {
                                content: question.to_string(),
                            });
                        }
                        return;
                    }
                }
            }
        }
    }

    if let Some(cb) = callbacks {
        cb.on_assistant_message("达到最大工具步数限制，已停止当前回合。");
    }
    append_runtime_message(ChatMessage::Assistant {
        content: "达到最大工具步数限制，已停止当前回合。".to_string(),
    });
}

fn get_messages_with_system() -> Vec<ChatMessage> {
    let messages_without_system = runtime_messages();
    let mut messages = Vec::with_capacity(messages_without_system.len() + 1);
    messages.push(ChatMessage::System {
        content: build_system_prompt(),
    });
    messages.extend(messages_without_system);
    messages
}
