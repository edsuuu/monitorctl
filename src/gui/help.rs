pub(super) const SECTIONS: [&str; 6] = ["Imagem", "Jogo", "AlienVision", "Conforto", "Cor e entrada", "Placa de vídeo"];

pub(super) fn section(name: &str) -> &'static str {
    match name {
        "Tempo de resposta" | "Dark Stabilizer" | "Game Enhancement" | "Posição do contador" | "Smart HDR" | "Overdrive" | "Shadow Control" | "Game Color" | "Low Input Lag"
        | "Dial Point (mira)" | "Frame Counter" | "HDR" => "Jogo",
        "Saturação (GPU)" => "Placa de vídeo",
        "AlienVision" => "AlienVision",
        vision_control if vision_control.ends_with("· V1") => "AlienVision",
        "Low Blue" |"Gamma" | "DCR" | "DCB" | "Clean Vision" => "Conforto",
        "Ganho vermelho" | "Ganho verde" | "Ganho azul" | "Nível de preto R" | "Nível de preto G" | "Nível de preto B" | "Entrada" | "Volume" | "Mudo" | "Idioma do OSD" => "Cor e entrada",
        _ => "Imagem",
    }
}

/// What each control does, with an example (the "?" button).
pub(super) fn help(name: &str) -> &'static str {
    match name {
        "Brilho" => "Luz de fundo do painel.\nEx.: 20–30 à noite, 70–100 de dia.",
        "Contraste" => "Diferença entre claro e escuro. Alto demais estoura os brancos.\nEx.: 50–75 costuma ser o ponto certo.",
        "Preset de cor" | "Temperatura de cor" => "Tom do branco.\nWarm/6500K = amarelado, confortável · Cool/9300K = azulado · sRGB = cores fiéis (web/fotos) · User/Custom libera os ganhos RGB.",
        "Temp. de cor (passos)" => "Ajuste fino da temperatura de cor em passos (MCCS 0x0C).",
        "Ganho vermelho" | "Ganho verde" | "Ganho azul" => "Intensidade de cada cor, vale no modo User/Custom.\nEx.: baixar o azul deixa a tela mais quente.",
        "Nível de preto R" | "Nível de preto G" | "Nível de preto B" => "Nível de preto de cada canal (offset).",
        "Entrada" => "Troca a entrada de vídeo do monitor.\nCuidado: numa entrada sem o PC, este app perde o controle até você voltar pelo menu do monitor.",
        "Volume" => "Volume do alto-falante / saída de fone do próprio monitor.",
        "Mudo" => "Silencia o áudio do monitor.",
        "Escala da imagem" | "Proporção" => "Como uma resolução menor é esticada.\nEx.: 1:1 mostra sem escalar (bom para 4:3 em jogo competitivo) · Tela cheia estica.",
        "Nitidez" => "Realce de bordas. Alto demais cria halos ao redor de textos.",
        "Saturação" => "Intensidade das cores. 0 = preto e branco.",
        "Saturação (GPU)" => "Saturação aplicada pela placa de vídeo (Vibração digital da NVIDIA) no sinal que sai para esta tela.\nFunciona até na TV, que não aceita DDC/CI. 50 = padrão do driver.\nEx.: 65–80 deixa as cores mais vivas em jogos.",
        "Matiz" => "Gira o tom de todas as cores.",
        "Idioma do OSD" => "Idioma do menu na tela do monitor.",
        "Modo de imagem" => "Preset de imagem do fabricante (Jogos, Filme, Esportes...).",
        "Overdrive" => "Acelera a troca dos pixels (menos rastro/ghosting em movimento).\nEx.: Médio é um bom início em 240 Hz; Forte/Boost pode criar contorno claro (overshoot).",
        "Shadow Control" => "Clareia áreas escuras sem estourar o resto.\nEx.: subir para enxergar inimigos em cantos escuros.",
        "Game Color" => "Saturação extra para jogos.\nEx.: 12–15 deixa as cores mais vivas.",
        "Low Input Lag" => "Desliga processamentos de imagem para reduzir o atraso entre mouse e tela.",
        "Dial Point (mira)" => "Desenha uma mira fixa no centro da tela, por cima de qualquer jogo.",
        "Frame Counter" => "Mostra os FPS/Hz atuais num canto da tela.",
        "Low Blue" => "Reduz luz azul (tela amarelada) para cansar menos a vista.\nEx.: Leitura é o mais forte.",
        "Modo ECO" => "Modos de brilho/contraste da AOC (Text, Internet, Game, Movie, Sport, Reading).\nSó vale com o Preset em Desligado.",
        "Gamma" => "Curva de gama. Gamma 1 ≈ 2.2 (padrão); os outros clareiam ou escurecem os meios-tons.",
        "DCR" => "Contraste dinâmico: ajusta a luz de fundo conforme a cena. O brilho pode 'respirar'.",
        "DCB" => "Realce de cores da AOC (pele, céu azul, verde).",
        "HDR" => "Modo de imagem HDR simulado do monitor.",
        "Clean Vision" => "Redução de ruído da imagem.",
        "Preset" => "Modo de imagem pronto do monitor.\nFPS clareia sombras · MOBA/RTS, RPG, Racing e SPORTS realçam cor · Game/Gamer 1–3 são perfis seus · sRGB = cores fiéis.",
        "Tempo de resposta" => "Overdrive do Alienware.\nExtreme = menos rastro, pode criar halos · Fast = imagem mais limpa.",
        "Dark Stabilizer" => "Clareia áreas escuras para enxergar em jogos escuros.\nEx.: Nível 2–3 em jogos de tiro à noite.",
        "Game Enhancement" => "Mostra um contador de FPS ou um timer (30–90 min) na tela.",
        "Posição do contador" => "Canto da tela onde aparecem o contador de FPS e o timer.",
        "Smart HDR" => "Modo HDR do monitor (Desktop, Movie, Game).\nSó tem efeito com o HDR ligado no Windows.",
        "AlienVision" => "Filtros de visão do Alienware para jogos (Night clareia, Chroma muda cores, Radar...) com mira.\nLiga um dos 3 perfis Vision.",
        vision_control if vision_control.ends_with("· V1") => "Configuração do perfil Vision 1 do AlienVision.\nAtive em AlienVision → Vision 1 para ver o efeito.",
        _ => "",
    }
}
