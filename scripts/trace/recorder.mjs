// Records the schema service and validation calls of the upstream tests.
// See run.mjs.
import { JSONSchemaService } from 'LIB/services/jsonSchemaService.js';
import { JSONValidation } from 'LIB/services/jsonValidation.js';
import { JSONDocument } from 'LIB/parser/jsonParser.js';
import { schemaContributions } from 'LIB/services/configuration.js';
import { AsyncLocalStorage } from 'node:async_hooks';
import fs from 'node:fs';
import path from 'node:path';

// Calls made inside a recorded call, also in its promise callbacks, are not recorded.
const inner = new AsyncLocalStorage();

const fixturesDir = new URL('../../../src/test/fixtures/', import.meta.url);
const fixtures = new Map();
for (const f of fs.readdirSync(fixturesDir, { recursive: true })) {
	const name = f.split(path.sep).join('/');
	const p = new URL(name, fixturesDir);
	if (fs.statSync(p).isFile()) {
		fixtures.set(fs.readFileSync(p, 'utf8'), name);
	}
}

export const tests = [];
export let current = null;
export function startTest(name) {
	current = { name, ops: [] };
	tests.push(current);
}

const ids = new WeakMap();
let nextId = 0;
function idOf(obj) {
	if (!ids.has(obj)) {
		ids.set(obj, nextId++);
	}
	return ids.get(obj);
}

// Dumps a value as JSON. A cycle becomes "$circular".
export function dump(value, limit = 400000) {
	let size = 0;
	const stack = [];
	const walk = v => {
		if (++size > limit) {
			throw new Error('too large');
		}
		if (v === undefined) {
			return null;
		}
		if (v === null || typeof v !== 'object') {
			return v;
		}
		if (stack.includes(v)) {
			return '$circular';
		}
		stack.push(v);
		let out;
		if (Array.isArray(v)) {
			out = v.map(walk);
		} else {
			out = {};
			for (const k of Object.keys(v)) {
				if (v[k] !== undefined) {
					out[k] = walk(v[k]);
				}
			}
		}
		stack.pop();
		return out;
	};
	try {
		return walk(value);
	} catch (e) {
		return '$toolarge';
	}
}

function record(op, fn) {
	if (inner.getStore() || !current) {
		return fn();
	}
	const entry = { ...op };
	current.ops.push(entry);
	let result;
	try {
		result = inner.run(true, fn);
	} catch (e) {
		entry.threw = String(e);
		throw e;
	}
	return { entry, result };
}

// Wraps the request service and path resolver of a service on first use.
function track(svc) {
	if (svc.__traced) {
		return;
	}
	svc.__traced = true;
	const svcId = idOf(svc);
	const test = () => current;
	if (svc.requestService) {
		const inner = svc.requestService;
		svc.requestService = uri => {
			const entry = { uri };
			test().ops.push({ op: 'request', svc: svcId, ...entry });
			const r = test().ops[test().ops.length - 1];
			return Promise.resolve().then(() => inner(uri)).then(
				content => { if (fixtures.has(content)) { r.fixture = fixtures.get(content); } else { r.content = content; } return content; },
				error => {
					r.error = { message: typeof error?.message === 'string' ? error.message : String(error), code: typeof error?.code === 'number' ? error.code : undefined };
					throw error;
				});
		};
	} else {
		test().ops.push({ op: 'noRequestService', svc: svcId });
	}
	if (svc.contextService) {
		const inner = svc.contextService.resolveRelativePath;
		svc.contextService = {
			resolveRelativePath: (rel, res) => {
				const out = inner(rel, res);
				test().ops.push({ op: 'resolvePath', svc: svcId, rel, res, out });
				return out;
			}
		};
	} else {
		test().ops.push({ op: 'noContextService', svc: svcId });
	}
	if (svc.fallbackSchemaProvider) {
		const inner = svc.fallbackSchemaProvider;
		svc.fallbackSchemaProvider = resource => {
			const out = inner(resource);
			test().ops.push({ op: 'fallback', svc: svcId, resource, out: out ?? null });
			return out;
		};
	}
}

// Upstream mutates schema objects that are passed in. Some tests pass one object to
// several calls. Each call gets a copy, as the server gives it fresh objects.
const fresh = v => v && typeof v === 'object' ? structuredClone(v) : v;

const diag = d => dump(d);
const resolved = r => r === undefined ? null : {
	errors: r.errors.map(diag),
	warnings: r.warnings.map(diag),
	schemaDraft: r.schemaDraft ?? null,
	vocabularies: r.activeVocabularies ? [...r.activeVocabularies.entries()] : null,
	schema: dump(r.schema),
};
const doc = (textDoc, jsonDoc) => ({ uri: textDoc.uri, text: textDoc.getText(), comments: jsonDoc.comments.length > 0 });
// getSchemaForResource only reads the root `$schema` string.
const jsonDocOnly = jsonDoc => {
	const p = jsonDoc.root?.type === 'object' ? jsonDoc.root.properties.find(p => p.keyNode.value === '$schema' && p.valueNode?.type === 'string') : undefined;
	return { schema: p ? p.valueNode.value : null };
};

function patchAsync(proto, name, toOp, toResult, copyArg = -1) {
	const orig = proto[name];
	proto[name] = function (...args) {
		if (copyArg >= 0 && !inner.getStore()) {
			args[copyArg] = fresh(args[copyArg]);
		}
		if (this instanceof JSONSchemaService) {
			track(this);
		} else if (this.jsonSchemaService) {
			track(this.jsonSchemaService);
		}
		const r = record(toOp(this, ...args), () => orig.apply(this, args));
		if (!r || !r.entry) {
			return r;
		}
		const { entry, result } = r;
		if (result && typeof result.then === 'function') {
			return result.then(v => { entry.result = toResult(v, ...args); return v; }, e => { entry.rejected = String(e); throw e; });
		}
		entry.result = toResult(result, ...args);
		return result;
	};
}

const S = JSONSchemaService.prototype;
const origContributions = S.setSchemaContributions;
S.setSchemaContributions = function (c) {
	track(this);
	const r = record({ op: 'setSchemaContributions', svc: idOf(this), builtin: c === schemaContributions, contributions: c === schemaContributions ? null : dump(c) },
		() => origContributions.call(this, c === schemaContributions ? c : fresh(c)));
	return r && r.entry ? r.result : r;
};
patchAsync(S, 'registerExternalSchema', (svc, config) => ({ op: 'registerExternalSchema', svc: idOf(svc), config: dump(config) }), (handle) => {
	const orig = handle.getResolvedSchema;
	const svc = handle.service;
	handle.getResolvedSchema = function () {
		const r = record({ op: 'getResolvedSchema', svc: idOf(svc), id: handle.uri }, () => orig.apply(this));
		if (!r || !r.entry) {
			return r;
		}
		return r.result.then(v => { r.entry.result = resolved(v); return v; });
	};
	return null;
}, 0);
patchAsync(S, 'clearExternalSchemas', svc => ({ op: 'clearExternalSchemas', svc: idOf(svc) }), () => null);
patchAsync(S, 'getResolvedSchema', (svc, id) => ({ op: 'getResolvedSchema', svc: idOf(svc), id }), resolved);
patchAsync(S, 'getSchemaForResource', (svc, resource, jsonDoc) => ({ op: 'getSchemaForResource', svc: idOf(svc), resource, doc: jsonDoc ? jsonDocOnly(jsonDoc) : null }), resolved);
patchAsync(S, 'onResourceChange', (svc, uri) => ({ op: 'onResourceChange', svc: idOf(svc), uri }), v => v);
patchAsync(S, 'getMatchingSchemas', (svc, textDoc, jsonDoc, schema) => ({ op: 'getMatchingSchemas', svc: idOf(svc), doc: doc(textDoc, jsonDoc), schema: schema ? dump(schema) : null }),
	v => v.map(m => ({ offset: m.node.offset, length: m.node.length, inverted: !!m.inverted })), 2);

const V = JSONValidation.prototype;
patchAsync(V, 'configure', (v, raw) => ({ op: 'configureValidation', validation: idOf(v), svc: idOf(v.jsonSchemaService), raw: dump(raw) }), () => null);
patchAsync(V, 'doValidation', (v, textDoc, jsonDoc, settings, schema) => ({ op: 'doValidation', validation: idOf(v), svc: idOf(v.jsonSchemaService), doc: doc(textDoc, jsonDoc), settings: dump(settings), schema: schema === undefined ? null : dump(schema) }), r => r.map(diag), 3);
patchAsync(V, 'getLanguageStatus', (v, textDoc, jsonDoc) => ({ op: 'getLanguageStatus', validation: idOf(v), svc: idOf(v.jsonSchemaService), doc: doc(textDoc, jsonDoc) }), r => r);

const D = JSONDocument.prototype;
patchAsync(D, 'validate', (jsonDoc, textDoc, schema, severity, draft, vocabs) => ({ op: 'validate', doc: doc(textDoc, jsonDoc), schema: dump(schema), severity: severity ?? null, draft: draft ?? null, vocabularies: vocabs ? [...vocabs.entries()] : null }), r => r === undefined ? null : r.map(diag));
