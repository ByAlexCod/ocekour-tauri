import type { McpStatus } from '../../../shared/api';

export type ClientId = 'claude' | 'qwen' | 'vibe' | 'other' | 'headless';

export interface Snippet {
  title: string;
  note: string;
  text: string;
}

export const MCP_CLIENTS: { id: ClientId; label: string }[] = [
  { id: 'claude', label: 'Claude Code' },
  { id: 'qwen', label: 'Qwen Code' },
  { id: 'vibe', label: 'Mistral Vibe' },
  { id: 'other', label: 'Autres clients' },
  { id: 'headless', label: 'Sans interface' },
];

/** Configuration à copier pour brancher un client MCP (les formats diffèrent d'un client à l'autre). */
export function mcpSnippets(s: Pick<McpStatus, 'url' | 'token' | 'executable'>, client: ClientId): Snippet[] {
  const bearer = `Bearer ${s.token}`;
  const exe = s.executable;
  const json = (value: unknown) => JSON.stringify(value, null, 2);
  switch (client) {
    case 'claude':
      return [
        {
          title: 'Commande (terminal)',
          note: 'Serveur actif requis : case ci-dessus cochée, ou Ocekour lancé avec --mcp-http. Vérifier : claude mcp list.',
          text: `claude mcp add --transport http ocekour ${s.url} --header "Authorization: ${bearer}"`,
        },
      ];
    case 'qwen':
      return [
        {
          title: 'Commande (terminal)',
          note: 'Portée utilisateur par défaut (%USERPROFILE%\\.qwen\\settings.json) ; « -s project » pour le seul projet courant. Vérifier : qwen mcp list.',
          text: `qwen mcp add --transport http ocekour ${s.url} -H "Authorization: ${bearer}"`,
        },
        {
          title: 'Ou dans settings.json',
          note: 'Qwen Code attend « httpUrl » (et non « url », réservé à l’ancien transport SSE) : sans cela, la connexion échoue.',
          text: json({ mcpServers: { ocekour: { httpUrl: s.url, headers: { Authorization: bearer } } } }),
        },
      ];
    case 'vibe':
      return [
        {
          title: 'Commande (terminal)',
          note: 'Enregistré dans %USERPROFILE%\\.vibe\\config.toml. Transport « streamable-http » obligatoire (sinon Vibe tente une connexion OAuth).',
          text: `vibe mcp add ocekour --url ${s.url} --transport streamable-http --header "Authorization=${bearer}"`,
        },
        {
          title: 'Ou dans config.toml',
          note: 'Les outils apparaissent dans Vibe préfixés par « ocekour_ » (ex. ocekour_logs_lire).',
          text: [
            '[[mcp_servers]]',
            'name = "ocekour"',
            'transport = "streamable-http"',
            `url = "${s.url}"`,
            '',
            '[mcp_servers.auth]',
            'type = "static"',
            `headers = { "Authorization" = "${bearer}" }`,
          ].join('\n'),
        },
      ];
    case 'other':
      return [
        {
          title: 'Configuration JSON (HTTP)',
          note: 'Format « mcpServers » courant (Cursor, VS Code…). Les clients dérivés de Gemini CLI, comme Qwen Code, attendent « httpUrl » à la place de « type » + « url ».',
          text: json({ mcpServers: { ocekour: { type: 'http', url: s.url, headers: { Authorization: bearer } } } }),
        },
      ];
    case 'headless':
      return [
        {
          title: 'HTTP en arrière-plan (recommandé)',
          note: 'À lancer au démarrage de la session (raccourci, tâche planifiée) : même adresse et même jeton, sans fenêtre. Puis configurez le client comme dans les autres onglets.',
          text: `"${exe}" --mcp-http`,
        },
        {
          title: 'stdio : le client lance Ocekour (Claude Code, Qwen Code, Mistral Vibe…)',
          note: "Pas de jeton : c'est le client qui démarre Ocekour. Si aucune réponse avec l'exe portable, préférez le mode HTTP.",
          text: json({ mcpServers: { ocekour: { command: exe, args: ['--mcp'] } } }),
        },
      ];
  }
}
