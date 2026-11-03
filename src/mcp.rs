// ---------- MCP: `monitorctl mcp` (stdio, JSON-RPC por linha). Cada ferramenta é um comando da CLI. ----------

use crate::log::{Logged, log};
use serde_json::{Value, json};
use std::io::Write;

fn tools() -> Value {
    let string = |description: &str| json!({"type": "string", "description": description});
    let object = |properties: Value, required: &[&str]| json!({"type": "object", "properties": properties, "required": required});
    json!([
        {"name": "listar", "description": "Lista as telas (● ligada, ○ desligada, conexão HDMI/DisplayPort), os monitores com DDC/CI e as saídas de áudio (* = padrão). Use primeiro para saber os nomes.",
         "inputSchema": object(json!({}), &[])},
        {"name": "telas", "description": "Liga/desliga telas, como o Win+P mas por tela. Nunca deixa zero telas ligadas. Não muda o áudio.",
         "inputSchema": object(json!({
            "acao": {"type": "string", "enum": ["on", "off", "only", "all"], "description": "on = ligar, off = desligar, only = deixar só estas ligadas, all = ligar todas"},
            "telas": {"type": "array", "items": {"type": "string"}, "description": "parte do nome de cada tela, ex: [\"samsung\"]; vazio para all"}}), &["acao"])},
        {"name": "perfil", "description": "Perfis de telas salvos pelo usuário (ex.: 'Só TV', 'Monitores'). Sem 'nome', lista os perfis; com 'nome', liga só as telas daquele perfil. Não muda o áudio.",
         "inputSchema": object(json!({"nome": string("nome do perfil, ex: só tv")}), &[])},
        {"name": "controles","description": "Todos os controles de um monitor (brilho, contraste, preset, overdrive...) com valor atual, máximo e opções. Use antes de ajustar.",
         "inputSchema": object(json!({"monitor": string("parte do nome, ex: aw ou 25g3")}), &["monitor"])},
        {"name": "ler", "description": "Lê um controle do monitor.",
         "inputSchema": object(json!({"monitor": string("parte do nome do monitor"), "controle": string("nome do controle (ex: brilho, preset) ou código VCP hex")}), &["monitor", "controle"])},
        {"name": "ajustar", "description": "Muda um controle do monitor na hora (persiste no monitor). Cuidado: mudar 'Entrada' tira a imagem do PC daquele monitor.",
         "inputSchema": object(json!({"monitor": string("parte do nome do monitor"), "controle": string("nome do controle (ex: brilho, preset, overdrive) ou código VCP hex"), "valor": string("número ou nome da opção, ex: 30, FPS, Desligado")}), &["monitor", "controle", "valor"])},
        {"name": "saturacao_gpu", "description": "Lê ou muda a saturação aplicada pela placa de vídeo NVIDIA (Vibração digital) numa tela, 0-100, 50 = padrão. Funciona até na TV, que não aceita DDC/CI. Não persiste após reiniciar o PC.",
         "inputSchema": object(json!({"tela": string("parte do nome da tela"), "valor": {"type": "integer", "minimum": 0, "maximum": 100, "description": "omitir para só ler"}}), &["tela"])},
        {"name": "taxa_atualizacao", "description": "Lê ou troca a taxa de atualização (Hz) de uma tela, entre as que a placa de vídeo oferece na resolução atual. Sem 'hz', só lista a atual e as opções.",
         "inputSchema": object(json!({"tela": string("parte do nome da tela"), "hz": {"type": "integer", "description": "omitir para só ler, ex: 240"}}), &["tela"])},
        {"name": "audio", "description": "Define a saída de áudio padrão do sistema.",
         "inputSchema": object(json!({"saida": string("parte do nome da saída, ex: samsung, realtek")}), &["saida"])}
    ])
}

/// Tool call -> CLI argv.
fn cli_arguments(name: &str, arguments: &Value) -> Result<Vec<String>, String> {
    let argument = |key: &str| arguments.get(key).map(|value| value.as_str().map_or(value.to_string(), str::to_string)).ok_or(format!("falta o argumento '{key}'"));
    Ok(match name {
        "listar" => vec!["list".into()],
        "telas" => {
            let displays: Vec<String> = arguments.get("telas").and_then(|list| list.as_array()).map(|list| list.iter().filter_map(|value| value.as_str().map(str::to_string)).collect()).unwrap_or_default();
            [vec![argument("acao")?], displays].concat()
        }
        "perfil" => [vec!["perfil".into()], argument("nome").ok().into_iter().collect()].concat(),
        "controles" => vec!["caps".into(), argument("monitor")?],
        "ler" => vec!["get".into(), argument("monitor")?, argument("controle")?],
        "ajustar" => vec!["set".into(), argument("monitor")?, argument("controle")?, argument("valor")?],
        "saturacao_gpu" => [vec!["sat".into(), argument("tela")?], argument("valor").ok().into_iter().collect()].concat(),
        "taxa_atualizacao" => [vec!["hz".into(), argument("tela")?], argument("hz").ok().into_iter().collect()].concat(),
        "audio" => vec!["audio".into(), argument("saida")?],
        _ => return Err(format!("ferramenta desconhecida: {name}")),
    })
}

pub(crate) fn serve() -> Result<(), String> {
    log("mcp: iniciado");
    for line in std::io::stdin().lines() {
        let line = line.map_err(|error| error.to_string()).logged("mcp stdin")?;
        let Ok(request) = serde_json::from_str::<Value>(&line).map_err(|error| format!("{error}: {line}")).logged("mcp json") else { continue };
        let Some(id) = request.get("id").cloned() else { continue }; // notification
        let params = &request["params"];
        let result = match request["method"].as_str().unwrap_or_default() {
            "initialize" => Ok(json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "monitorctl", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Controla os monitores e telas deste PC: liga/desliga telas, brilho/contraste/presets via DDC/CI, saturação pela GPU, taxa de atualização (Hz) e saída de áudio. Use 'listar' e 'controles' antes de ajustar."
            })),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                log(format!("mcp {name} {}", params["arguments"]));
                let output = cli_arguments(name, &params["arguments"]).and_then(|arguments| crate::cli::run(&arguments)).logged(&format!("mcp {name}"));
                Ok(json!({"content": [{"type": "text", "text": output.as_deref().unwrap_or_else(|error| error)}], "isError": output.is_err()}))
            }
            "ping" => Ok(json!({})),
            method => Err(json!({"code": -32601, "message": format!("método desconhecido: {method}")})),
        };
        let message = match result {
            Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
            Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
        };
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{message}").and_then(|_| stdout.flush()).map_err(|error| error.to_string()).logged("mcp stdout")?;
    }
    log("mcp: stdin fechado, saindo");
    Ok(())
}
