// Opens deeply nested documents in the release servers and sends each request.
// Prints the time of each case, or the first request without an answer (exit
// code of the server, or a timeout after TIMEOUT ms, default 60000).
// Formatting is not sent: its output grows as the square of the depth.
// Usage: node scripts/deep-nesting.mjs <bin dir> [depth] [case ...]
// For example: node scripts/deep-nesting.mjs target/replay/release 100000
import { spawn } from 'node:child_process';

const cases = {
  json_array: ['json', 'json', (n) => '['.repeat(n)],
  json_object: ['json', 'json', (n) => '{"a":'.repeat(n)],
  json_closed: ['json', 'json', (n) => '['.repeat(n) + ']'.repeat(n)],
  json_mismatch: ['json', 'json', (n) => '[}'.repeat(n)],
  json_mismatch_colon: ['json', 'json', (n) => '{"a":]'.repeat(n)],
  json_mix: ['json', 'json', (n) => '[{"a":'.repeat(n)],
  css_rules: ['css', 'css', (n) => 'a{'.repeat(n)],
  css_parens: ['css', 'css', (n) => 'a{b:' + '('.repeat(n)],
  css_parens_closed: ['css', 'css', (n) => 'a{b:' + '('.repeat(n) + '1' + ')'.repeat(n) + '}'],
  css_functions: ['css', 'css', (n) => 'a{b:' + 'f('.repeat(n)],
  css_binary: ['css', 'css', (n) => 'a{b:' + '1+'.repeat(n) + '1}'],
  css_not: ['css', 'css', (n) => ':not('.repeat(n)],
  css_not_closed: ['css', 'css', (n) => ':not('.repeat(n) + 'a' + ')'.repeat(n) + '{}'],
  css_media: ['css', 'css', (n) => '@media ' + '('.repeat(n)],
  css_media_and: ['css', 'css', (n) => '@media ' + '(a) and '.repeat(n) + '(a){}'],
  css_supports: ['css', 'css', (n) => '@supports ' + '('.repeat(n)],
  css_supports_not: ['css', 'css', (n) => '@supports ' + 'not '.repeat(n) + '(a:b){}'],
  css_container: ['css', 'css', (n) => '@container ' + 'style('.repeat(n)],
  css_layer: ['css', 'css', (n) => '@layer a{'.repeat(n)],
  css_mismatch: ['css', 'css', (n) => 'a{)'.repeat(n)],
  css_mismatch_curly: ['css', 'css', (n) => 'a{b:(}'.repeat(n)],
  css_mismatch_bracket: ['css', 'css', (n) => 'a{b:(])'.repeat(n)],
  css_mix: ['css', 'css', (n) => 'a{b:' + '[('.repeat(n)],
  css_custom: ['css', 'css', (n) => 'a{--x:' + '{('.repeat(n)],
  css_url: ['css', 'css', (n) => 'a{b:' + 'url('.repeat(n)],
  css_unary: ['css', 'css', (n) => 'a{b:' + '-'.repeat(n) + '1}'],
  css_selectors: ['css', 'css', (n) => 'a '.repeat(n) + '{}'],
  css_comma: ['css', 'css', (n) => 'a{b:' + '1,'.repeat(n) + '1}'],
  css_attr: ['css', 'css', (n) => 'a{b:' + 'attr(x type('.repeat(n)],
  css_if: ['css', 'css', (n) => 'a{b:' + 'if(style(--x:'.repeat(n)],
  css_closers: ['css', 'css', (n) => 'a{'.repeat(n) + '}'.repeat(n) + 'a{'.repeat(n)],
  scss_rules: ['css', 'scss', (n) => 'a{'.repeat(n)],
  scss_binary: ['css', 'scss', (n) => '$a: ' + '1 + '.repeat(n) + '1;'],
  scss_interp: ['css', 'scss', (n) => 'a{b:' + '#{'.repeat(n)],
  scss_interp_closed: ['css', 'scss', (n) => 'a{b:' + '#{'.repeat(n) + '1' + '}'.repeat(n) + '}'],
  scss_map: ['css', 'scss', (n) => '$a: ' + '('.repeat(n)],
  scss_else: ['css', 'scss', (n) => '@if a {} ' + '@else if b {} '.repeat(n)],
  scss_nested_props: ['css', 'scss', (n) => 'a{' + 'b:{'.repeat(n)],
  scss_include: ['css', 'scss', (n) => 'a{@include m(' + 'f('.repeat(n)],
  scss_mismatch: ['css', 'scss', (n) => 'a{#{)'.repeat(n)],
  scss_unary: ['css', 'scss', (n) => '$a: ' + '-'.repeat(n) + '1;'],
  scss_not: ['css', 'scss', (n) => '$a: ' + 'not '.repeat(n) + '1;'],
  scss_each: ['css', 'scss', (n) => '@each $a in b {'.repeat(n)],
  scss_media: ['css', 'scss', (n) => 'a{@media b{'.repeat(n)],
  scss_placeholder: ['css', 'scss', (n) => '%a{'.repeat(n)],
  less_rules: ['css', 'less', (n) => 'a{'.repeat(n)],
  less_binary: ['css', 'less', (n) => '@a: ' + '1 + '.repeat(n) + '1;'],
  less_guard: ['css', 'less', (n) => '.m() when ' + '('.repeat(n)],
  less_guard_and: ['css', 'less', (n) => '.m() when ' + '(a) and '.repeat(n) + '(a){}'],
  less_mixin: ['css', 'less', (n) => '.m('.repeat(n)],
  less_mixin_closed: ['css', 'less', (n) => '.m('.repeat(n) + ')'.repeat(n) + ';'],
  less_detached: ['css', 'less', (n) => '@d: {'.repeat(n)],
  less_mismatch: ['css', 'less', (n) => '.m(){)'.repeat(n)],
  less_interp: ['css', 'less', (n) => '@{'.repeat(n)],
  less_unary: ['css', 'less', (n) => '@a: ' + '-'.repeat(n) + '1;'],
  less_escape: ['css', 'less', (n) => 'a{b:' + '~"x" '.repeat(n) + '}'],
  html_div: ['html', 'html', (n) => '<div>'.repeat(n)],
  html_closed: ['html', 'html', (n) => '<div>'.repeat(n) + '</div>'.repeat(n)],
  html_void: ['html', 'html', (n) => '<div><br>'.repeat(n)],
  html_mismatch: ['html', 'html', (n) => '<div></p>'.repeat(n)],
  html_self: ['html', 'html', (n) => '<div/><a>'.repeat(n)],
  html_style: ['html', 'html', (n) => '<style>' + 'a{'.repeat(n) + '</style>'],
  html_style_attr: ['html', 'html', (n) => '<div style="b:' + '('.repeat(n) + '">'],
  html_style_attr_curly: ['html', 'html', (n) => '<div style="a:' + '{'.repeat(n) + '">'],
  // The JavaScript mode (tsrs) has no limit: slow above a few thousand levels.
  html_script: ['html', 'html', (n) => '<script>' + '['.repeat(n) + '</script>'],
};

const [binDir, depth = '100000', ...names] = process.argv.slice(2);
const timeout = Number(process.env.TIMEOUT ?? 60000);

function run(name, depth) {
  const [server, languageId, generate] = cases[name];
  const text = generate(depth);
  const uri = `file:///w/a.${languageId}`;
  const child = spawn(`${binDir}/vscode-${server}-language-server.exe`, ['--stdio'], {
    stdio: ['pipe', 'pipe', 'ignore'],
  });
  let buffer = Buffer.alloc(0);
  const waiting = new Map();
  let exitCode;
  child.on('exit', (code) => {
    exitCode = code;
    for (const done of waiting.values()) done(null);
  });
  const send = (message) => {
    if (exitCode !== undefined) return;
    const body = Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...message }));
    child.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
    child.stdin.write(body);
  };
  child.stdout.on('data', (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    for (;;) {
      const header = buffer.indexOf('\r\n\r\n');
      if (header < 0) return;
      const length = Number(/Content-Length: (\d+)/.exec(buffer.subarray(0, header))[1]);
      if (buffer.length < header + 4 + length) return;
      const message = JSON.parse(buffer.subarray(header + 4, header + 4 + length));
      buffer = buffer.subarray(header + 4 + length);
      if (message.method && message.id !== undefined) {
        const result = message.method === 'workspace/configuration' ? message.params.items.map(() => null) : null;
        send({ id: message.id, result });
      } else if (waiting.has(message.id)) {
        waiting.get(message.id)(message);
        waiting.delete(message.id);
      }
    }
  });
  let id = 0;
  const request = (method, params) =>
    new Promise((done) => {
      if (exitCode !== undefined) return done(null);
      const i = ++id;
      const timer = setTimeout(() => done('timeout'), timeout);
      waiting.set(i, (message) => {
        clearTimeout(timer);
        done(message);
      });
      send({ id: i, method, params });
    });

  const textDocument = { uri };
  const middle = { line: 0, character: Math.floor(text.length / 2) };
  const end = { line: 0, character: text.length };
  const requests = [
    ['textDocument/diagnostic', { textDocument }],
    ['textDocument/documentSymbol', { textDocument }],
    ['textDocument/foldingRange', { textDocument }],
    ['textDocument/selectionRange', { textDocument, positions: [middle, end] }],
    ['textDocument/hover', { textDocument, position: middle }],
    ['textDocument/completion', { textDocument, position: middle }],
    ['textDocument/completion', { textDocument, position: end }],
    ['textDocument/documentColor', { textDocument }],
    ['textDocument/documentLink', { textDocument }],
    ['textDocument/documentHighlight', { textDocument, position: middle }],
    ['textDocument/definition', { textDocument, position: middle }],
    ['textDocument/references', { textDocument, position: middle, context: { includeDeclaration: true } }],
    ['textDocument/rename', { textDocument, position: middle, newName: 'x' }],
    ['textDocument/codeAction', { textDocument, range: { start: { line: 0, character: 0 }, end }, context: { diagnostics: [] } }],
    ['textDocument/semanticTokens/full', { textDocument }],
    ['textDocument/linkedEditingRange', { textDocument, position: middle }],
  ];

  return (async () => {
    const start = Date.now();
    const capabilities = { textDocument: { documentSymbol: { hierarchicalDocumentSymbolSupport: true }, diagnostic: {} } };
    await request('initialize', { processId: null, rootUri: null, capabilities, initializationOptions: { provideFormatter: true } });
    send({ method: 'initialized', params: {} });
    send({ method: 'textDocument/didOpen', params: { textDocument: { uri, languageId, version: 1, text } } });
    for (const [method, params] of requests) {
      const response = await request(method, params);
      if (response === null || response === 'timeout') {
        child.kill();
        return `${name}: ${response === null ? `exit ${exitCode}` : 'timeout'} at ${method}`;
      }
    }
    await request('shutdown', null);
    send({ method: 'exit' });
    return `${name}: ${Date.now() - start} ms`;
  })();
}

for (const name of names.length ? names : Object.keys(cases)) {
  console.log(await run(name, Number(depth)));
}
