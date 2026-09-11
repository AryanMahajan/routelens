// Plain JavaScript, and an alias export.
const handler = () => Response.json({ ok: true });

export { handler as GET, handler as HEAD };
