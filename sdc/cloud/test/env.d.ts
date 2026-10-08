// `exports` from cloudflare:workers is typed per project; this project's Worker is the default export.
declare namespace Cloudflare {
  interface Env {
    DB: D1Database;
    HUB: DurableObjectNamespace<import('../src/hub').Hub>;
  }

  interface Exports {
    default: { fetch(input: string | Request, init?: RequestInit): Promise<Response> };
  }
}

declare module '*?raw' {
  const text: string;
  export default text;
}

declare module '*.sql?raw' {
  const sql: string;
  export default sql;
}
