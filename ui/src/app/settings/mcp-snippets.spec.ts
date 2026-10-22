import { MCP_CLIENTS, mcpSnippets } from './mcp-snippets';

const status = { url: 'http://127.0.0.1:7801/mcp', token: 'abc', executable: 'C:\\Ocekour\\ocekour.exe' };

describe('mcpSnippets', () => {
  it('donne au moins un extrait par client', () => {
    for (const c of MCP_CLIENTS) expect(mcpSnippets(status, c.id).length).toBeGreaterThan(0);
  });

  it('Claude Code : commande avec en-tête Bearer', () => {
    expect(mcpSnippets(status, 'claude')[0].text).toBe(
      'claude mcp add --transport http ocekour http://127.0.0.1:7801/mcp --header "Authorization: Bearer abc"',
    );
  });

  it('Qwen Code : httpUrl et non url', () => {
    const json = JSON.parse(mcpSnippets(status, 'qwen')[1].text);
    expect(json.mcpServers.ocekour).toEqual({ httpUrl: status.url, headers: { Authorization: 'Bearer abc' } });
  });

  it('Mistral Vibe : transport streamable-http et auth statique', () => {
    const toml = mcpSnippets(status, 'vibe')[1].text;
    expect(toml).toContain('transport = "streamable-http"');
    expect(toml).toContain('headers = { "Authorization" = "Bearer abc" }');
  });

  it('sans interface : exécutable avec --mcp-http et --mcp', () => {
    const [http, stdio] = mcpSnippets(status, 'headless');
    expect(http.text).toBe('"C:\\Ocekour\\ocekour.exe" --mcp-http');
    expect(JSON.parse(stdio.text).mcpServers.ocekour).toEqual({ command: status.executable, args: ['--mcp'] });
  });
});
