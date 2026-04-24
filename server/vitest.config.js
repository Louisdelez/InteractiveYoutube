// Vitest config — enables globals so `describe` / `it` / `expect`
// don't need to be imported in every file. Works with the CJS
// server codebase without conversion to ESM.
export default {
  test: {
    globals: true,
    include: ['__tests__/**/*.test.js'],
    environment: 'node',
  },
};
