use serde_json::Value;

use crate::tool::{ToolContext, ToolRegistry};
use crate::types::{AgentStep, ChatMessage, ModelAdapter};

pub trait AgentTurnCallbacks: Send {
    fn on_tool_start(&mut self, _tool_name: &str, _input: &Value) {}
    fn on_tool_result(&mut self, _tool_name: &str, _output: &str, _is_error: bool) {}
    fn on_assistant_message(&mut self, _content: &str) {}
    fn on_progress_message(&mut self, _content: &str) {}
}

fn is_empty_assistant_response(content: &str) -> bool {
    content.trim().is_empty()
}

fn format_diagnostics(
    stop_reason: Option<&str>,
    block_types: Option<&[String]>,
    ignored: Option<&[String]>,
) -> String {
    let mut parts = vec![];
    if let Some(s) = stop_reason {
        if !s.is_empty() {
            parts.push(format!("stop_reason={s}"));
        }
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

pub async fn run_agent_turn(
    model: &dyn ModelAdapter,
    tools: &ToolRegistry,
    mut messages: Vec<ChatMessage>,
    context: ToolContext,
    max_steps: Option<usize>,
    mut callbacks: Option<&mut (dyn AgentTurnCallbacks + Send)>,
) -> Vec<ChatMessage> {
    let mut empty_retry = 0usize;
    let mut recover_retry = 0usize;
    let mut tool_error_count = 0usize;
    let mut saw_tool_result = false;

    let push_continue = |messages: &mut Vec<ChatMessage>, content: &str| {
        messages.push(ChatMessage::User {
            content: content.to_string(),
        });
    };

    let limit = max_steps.unwrap_or(64);

    for _ in 0..limit {
        let next = match model.next(&messages).await {
            Ok(step) => step,
            Err(err) => {
                if let Some(cb) = callbacks.as_deref_mut() {
                    cb.on_assistant_message(&format!("请求失败: {err}"));
                }
                messages.push(ChatMessage::Assistant {
                    content: format!("请求失败: {err}"),
                });
                return messages;
            }
        };

        match next {
            AgentStep::Assistant {
                content,
                kind: _,
                diagnostics,
            } => {
                let is_empty = is_empty_assistant_response(&content);

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
                        messages.push(ChatMessage::AssistantProgress { content: progress });
                        push_continue(&mut messages, "继续，从你刚才中断的位置直接执行下一步。");
                        continue;
                    }

                    if empty_retry < 2 {
                        empty_retry += 1;
                        push_continue(
                            &mut messages,
                            "上一条回复为空，请继续并给出下一步具体执行。",
                        );
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
                    messages.push(ChatMessage::Assistant { content: fallback });
                    return messages;
                }

                if let Some(cb) = callbacks.as_deref_mut() {
                    cb.on_assistant_message(&content);
                }
                messages.push(ChatMessage::Assistant { content });
                return messages;
            }
            AgentStep::ToolCalls {
                calls,
                content,
                content_kind,
                ..
            } => {
                if let Some(c) = content {
                    if content_kind.as_deref() == Some("progress") {
                        if let Some(cb) = callbacks.as_deref_mut() {
                            cb.on_progress_message(&c);
                        }
                        messages.push(ChatMessage::AssistantProgress { content: c });
                        push_continue(&mut messages, "继续，给出下一步工具调用或最终答案。");
                    } else {
                        if let Some(cb) = callbacks.as_deref_mut() {
                            cb.on_assistant_message(&c);
                        }
                        messages.push(ChatMessage::Assistant { content: c });
                    }
                }

                if calls.is_empty() {
                    continue;
                }

                for call in calls {
                    if let Some(cb) = callbacks.as_deref_mut() {
                        cb.on_tool_start(&call.tool_name, &call.input);
                    }
                    let result = tools
                        .execute(&call.tool_name, call.input.clone(), &context)
                        .await;
                    if let Some(cb) = callbacks.as_deref_mut() {
                        cb.on_tool_result(&call.tool_name, &result.output, !result.ok);
                    }
                    saw_tool_result = true;
                    if !result.ok {
                        tool_error_count += 1;
                    }

                    messages.push(ChatMessage::AssistantToolCall {
                        tool_use_id: call.id.clone(),
                        tool_name: call.tool_name.clone(),
                        input: call.input,
                    });
                    messages.push(ChatMessage::ToolResult {
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
                            messages.push(ChatMessage::Assistant {
                                content: question.to_string(),
                            });
                        }
                        return messages;
                    }
                }
            }
        }
    }

    if let Some(cb) = callbacks.as_deref_mut() {
        cb.on_assistant_message("达到最大工具步数限制，已停止当前回合。");
    }
    messages.push(ChatMessage::Assistant {
        content: "达到最大工具步数限制，已停止当前回合。".to_string(),
    });
    messages
}

#[allow(dead_code)]
fn _to_json_value(v: &str) -> Value {
    Value::String(v.to_string())
}
