use super::{Definition, Operation};

const OFF_ON: &[(u16, &str)] = &[(0, "Desligado"), (1, "Ligado")];

pub(super) const MCCS: &[Definition] = &[
    (Operation::Vcp(0x0C), "Temp. de cor (passos)", &[]),
    (Operation::Vcp(0x10), "Brilho", &[]),
    (Operation::Vcp(0x12), "Contraste", &[]),
    (Operation::Vcp(0x14), "Preset de cor", &[(1, "sRGB"), (2, "Nativo"), (3, "4000K"), (4, "5000K"), (5, "6500K"), (6, "7500K"), (7, "8200K"), (8, "9300K"), (9, "10000K"), (0x0A, "11500K"), (0x0B, "Usuário 1"), (0x0C, "Usuário 2"), (0x0D, "Usuário 3")]),
    (Operation::Vcp(0x16), "Ganho vermelho", &[]),
    (Operation::Vcp(0x18), "Ganho verde", &[]),
    (Operation::Vcp(0x1A), "Ganho azul", &[]),
    (Operation::Vcp(0x60), "Entrada", &[(0x01, "VGA 1"), (0x03, "DVI 1"), (0x0F, "DisplayPort 1"), (0x10, "DisplayPort 2"), (0x11, "HDMI 1"), (0x12, "HDMI 2"), (0x13, "HDMI 3"), (0x15, "USB-C 1"), (0x16, "USB-C 2"), (0x1B, "USB-C")]),
    (Operation::Vcp(0x62), "Volume", &[]),
    (Operation::Vcp(0x6C), "Nível de preto R", &[]),
    (Operation::Vcp(0x6E), "Nível de preto G", &[]),
    (Operation::Vcp(0x70), "Nível de preto B", &[]),
    (Operation::Vcp(0x86), "Escala da imagem", &[(1, "Sem escala"), (2, "Máxima, mantém proporção"), (3, "Máx. vertical, mantém proporção"), (4, "Máx. horizontal, mantém proporção"), (5, "Máx. vertical, distorce"), (6, "Máx. horizontal, distorce"), (7, "Expansão linear H"), (8, "Expansão linear H+V"), (9, "Squeeze"), (0x0A, "Expansão não linear")]),
    (Operation::Vcp(0x87), "Nitidez", &[]),
    (Operation::Vcp(0x8A), "Saturação", &[]),
    (Operation::Vcp(0x8D), "Mudo", &[(1, "Ligado"), (2, "Desligado")]),
    (Operation::Vcp(0x90), "Matiz", &[]),
    (Operation::Vcp(0xCC), "Idioma do OSD", &[(1, "中文 (trad.)"), (2, "English"), (3, "Français"), (4, "Deutsch"), (5, "Italiano"), (6, "日本語"), (7, "한국어"), (8, "Português (PT)"), (9, "Русский"), (0x0A, "Español"), (0x0B, "Svenska"), (0x0C, "Türkçe"), (0x0D, "中文 (simpl.)"), (0x0E, "Português (BR)"), (0x0F, "العربية"), (0x12, "Čeština"), (0x14, "Nederlands"), (0x16, "Suomi"), (0x1A, "Magyar"), (0x1E, "Polski")]),
    (Operation::Vcp(0xDC), "Modo de imagem", &[(0, "Padrão"), (1, "Produtividade"), (2, "Misto"), (3, "Filme"), (4, "Usuário"), (5, "Jogos"), (6, "Esportes"), (7, "Profissional")]),
];

/// AOC G-Menu 3.31 (monitorctl-rea/aoc-25g3zm.md).
pub(super) const AOC: &[Definition] = &[
    // game mode (0xDC) is what the OSD calls the preset; with it off, "Modo ECO" applies
    (Operation::Vcp(0xDC), "Preset", &[(0, "Desligado"), (0x0B, "FPS"), (0x0C, "RTS"), (0x0D, "Racing"), (0x0E, "Gamer 1"), (0x0F, "Gamer 2"), (0x10, "Gamer 3")]),
    (Operation::Vcp(0x14), "Temperatura de cor", &[(1, "sRGB"), (5, "Warm"), (6, "Normal"), (8, "Cool"), (0x0B, "User"), (0x0E, "AdobeRGB")]),
    (Operation::Vcp(0x86), "Proporção", &[(1, "1:1"), (2, "Tela cheia"), (5, "Aspecto"), (0x0B, "17\" 4:3"), (0x0C, "19\" 4:3"), (0x0D, "19\" 16:10"), (0x0E, "21.5\" 16:9"), (0x0F, "22\" 16:10"), (0x10, "23\" 16:9"), (0x11, "23.6\" 16:9"), (0x12, "24\" 16:9"), (0x13, "24\" 16:10")]),
    (Operation::Aoc(0x51, 0x0010), "Overdrive", &[(0, "Desligado"), (1, "Fraco"), (2, "Médio"), (3, "Forte"), (4, "Boost")]),
    (Operation::Aoc(0x58, 0x0800), "Shadow Control", &[]),
    (Operation::Aoc(0x57, 0x0400), "Game Color", &[]),
    (Operation::Aoc(0x55, 0x0100), "Low Input Lag", OFF_ON),
    (Operation::Aoc(0x5A, 0x4000), "Dial Point (mira)", OFF_ON),
    (Operation::Aoc(0x5B, 0x8000), "Frame Counter", &[(0, "Desligado"), (1, "Superior direito"), (2, "Inferior direito"), (3, "Inferior esquerdo"), (4, "Superior esquerdo")]),
    (Operation::Aoc(0x59, 0x1000), "Low Blue", &[(0, "Desligado"), (1, "Multimídia"), (2, "Internet"), (3, "Escritório"), (4, "Leitura")]),
    (Operation::Aoc(0x11, 0x0001), "Modo ECO", &[(0, "Standard"), (1, "Text"), (2, "Internet"), (3, "Game"), (4, "Movie"), (5, "Sport"), (6, "Reading"), (7, "Uniformity")]),
    (Operation::Aoc(0x12, 0x0002), "Gamma", &[(0, "Gamma 1"), (1, "Gamma 2"), (2, "Gamma 3"), (3, "Gamma 4"), (4, "Gamma 5")]),
    (Operation::Aoc(0x13, 0x0004), "DCR", OFF_ON),
    (Operation::Aoc(0x08, 0x0008), "DCB", &[(0, "Desligado"), (0x21, "Full Enhance"), (0x22, "Nature Skin"), (0x23, "Sky Blue"), (0x24, "Green Field"), (0x25, "Auto Detect"), (0x26, "Demo")]),
    (Operation::Aoc(0x52, 0x0020), "HDR", &[]), // names depend on the reported max, see hdr_names
    (Operation::Aoc(0x14, 0), "Clean Vision", &[(0, "Desligado"), (1, "Fraco"), (2, "Médio"), (3, "Forte")]),
    // 6-axis saturation/hue (F8 E1 71..7D) exist in G-Menu but the 25G3ZM never answers them (stale
    // reply, see monitorctl-rea/captures.md); probing them cost ~6 s per start, so they are not listed.
];

/// AlienVision bit field in Dell code 0xED.
const fn code_ed_bits(shift: u8, mask: u16) -> Operation {
    Operation::DellBits { code: 0xED, sel: 1 << 13, set: 0x8000, keep: 0x1FFF, shift, mask }
}
/// AlienVision bit field in Dell code 0xEC.
const fn code_ec_bits(shift: u8, mask: u16) -> Operation {
    Operation::DellBits { code: 0xEC, sel: 1 << 12, set: 0x4000, keep: 0x07FF, shift, mask }
}
const LEVELS: &[(u16, &str)] = &[(0, "Desligado"), (1, "Baixo"), (2, "Médio"), (3, "Alto")];

/// Alienware Command Center 6.14 (monitorctl-rea/alienware-aw2525hm.md).
pub(super) const ALIENWARE: &[Definition] = &[
    (Operation::Vcp(0x14), "", &[]), // presets are handled by DellPreset
    (Operation::Vcp(0xDC), "", &[]),
    (Operation::DellPreset, "Preset", &[(0, "Standard"), (32, "FPS"), (33, "MOBA/RTS"), (34, "RPG"), (47, "SPORTS"), (4, "Game 1"), (30, "Game 2"), (31, "Game 3"), (14, "Warm"), (18, "Cool"), (11, "sRGB"), (20, "Custom Color"), (127, "Indisponível (HDR)")]),
    (Operation::DellQ(0xF4, 0x2F), "Tempo de resposta", &[(0x20, "Extreme"), (0x21, "Super Fast"), (0x22, "Fast")]),
    (Operation::DellQ(0xF4, 0x3F), "Dark Stabilizer", &[(0x30, "Desligado"), (0x31, "Nível 1"), (0x32, "Nível 2"), (0x33, "Nível 3")]),
    (Operation::DellQ(0xF4, 0x1F), "Game Enhancement", &[(0x10, "Desligado"), (0x11, "Contador de FPS"), (0x13, "Timer 30 min"), (0x14, "Timer 40 min"), (0x15, "Timer 50 min"), (0x16, "Timer 60 min"), (0x17, "Timer 90 min")]),
    (Operation::DellQ(0xF4, 0xBF), "Posição do contador", &[(0xB1, "Superior direito"), (0xB2, "Superior esquerdo"), (0xB3, "Inferior direito"), (0xB4, "Inferior esquerdo")]),
    (Operation::DellQ(0xF4, 0x4F), "Smart HDR", &[(0x40, "Desligado"), (0x41, "Desktop"), (0x42, "Movie HDR"), (0x43, "Game HDR"), (0x45, "Custom Color HDR"), (0x4E, "Indisponível")]),
    (Operation::DellVision, "AlienVision", &[(0, "Desligado"), (1, "Vision 1"), (2, "Vision 2"), (3, "Vision 3")]),
    (code_ed_bits(10, 7), "Modo · V1", &[(0, "Desligado"), (1, "Night"), (2, "Chroma"), (3, "Radar")]),
    (code_ed_bits(8, 3), "Tamanho · V1", &[(0, "1"), (1, "2"), (2, "3"), (3, "4")]),
    (code_ed_bits(6, 3), "Nitidez · V1", OFF_ON),
    (code_ed_bits(2, 0xF), "Mira · V1", &[(0, "Desligada"), (1, "Ponto"), (2, "Cruz grande"), (3, "Barra/cruz pequena"), (4, "Cruz vazada pequena"), (5, "Custom 1"), (6, "Custom 2"), (7, "Custom 3"), (8, "Custom 4")]),
    (code_ed_bits(0, 3), "Cor da mira · V1", &[(0, "Branca"), (1, "Vermelha"), (2, "Verde"), (3, "Dinâmica")]),
    (code_ec_bits(5, 3), "Zoom · V1", LEVELS),
    (code_ec_bits(3, 3), "Transparência · V1", LEVELS),
    (code_ec_bits(0, 7), "Posição · V1", &[(0, "Centro"), (1, "Sup. esquerdo"), (2, "Sup. direito"), (3, "Inf. esquerdo"), (4, "Inf. direito")]),
];

/// Dell preset value (E2 numbering) -> (code, value) to write.
pub(super) const DELL_PRESET_WRITE: &[(u16, u8, u16)] = &[(0, 0xDC, 0x00), (4, 0xDC, 0x05), (11, 0x14, 0x01), (18, 0x14, 0x08), (14, 0x14, 0x0B), (20, 0x14, 0x0C), (30, 0xF0, 0x0D), (31, 0xF0, 0x0E), (32, 0xF0, 0x0F), (33, 0xF0, 0x10), (34, 0xF0, 0x11), (47, 0xF0, 0x13)];
