/** @type {import('jest').Config} */
export default {
  rootDir: ".",
  testEnvironment: "node",
  transform: {
    "^.+\\.tsx?$": [
      "ts-jest",
      {
        useESM: true,
        tsconfig: {
          jsx: "react-jsx",
          esModuleInterop: true,
          module: "ESNext",
          moduleResolution: "node",
          target: "ES2022",
          lib: ["ES2022", "DOM", "DOM.Iterable"],
        },
      },
    ],
  },
  extensionsToTreatAsEsm: [".ts", ".tsx"],
  moduleNameMapper: {
    "^(\\.{1,2}/.*)\\.js$": "$1",
    "^@opensymphony/(.+)$": "<rootDir>/packages/$1/src/index.ts",
  },
  testMatch: ["**/__tests__/**/*.test.ts", "**/__tests__/**/*.test.tsx"],
  testPathIgnorePatterns: ["<rootDir>/target/", "/\\.venv/"],
  modulePathIgnorePatterns: ["<rootDir>/target/", "/\\.venv/"],
  transformIgnorePatterns: ["/node_modules/"],
};
