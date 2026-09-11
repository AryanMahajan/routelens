export async function listUsers(_: { limit: number; search: string | null }) {
  return [];
}
export async function createUser(body: unknown) {
  return body;
}
export async function getUser(id: string) {
  return { id };
}
export async function deleteUser(_: string) {}
