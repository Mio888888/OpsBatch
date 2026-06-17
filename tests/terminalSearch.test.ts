import { findTerminalSearchMatches } from '../src/utils/terminalSearch';

function assertDeepEqual(actual: unknown, expected: unknown) {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`Expected ${expectedJson}, got ${actualJson}`);
  }
}

function testFindsMatchesCaseInsensitivelyAcrossRows() {
  const matches = findTerminalSearchMatches([
    { row: 3, text: 'Deploy OK' },
    { row: 4, text: 'error: service failed' },
    { row: 5, text: 'next ERROR marker' },
  ], 'error');

  assertDeepEqual(matches, [
    { row: 4, column: 0, length: 5 },
    { row: 5, column: 5, length: 5 },
  ]);
}

function testReturnsEveryOccurrenceOnSameRow() {
  const matches = findTerminalSearchMatches([
    { row: 8, text: 'foo bar foo' },
  ], 'foo');

  assertDeepEqual(matches, [
    { row: 8, column: 0, length: 3 },
    { row: 8, column: 8, length: 3 },
  ]);
}

function testIgnoresEmptyQueries() {
  const matches = findTerminalSearchMatches([
    { row: 1, text: 'anything' },
  ], '  ');

  assertDeepEqual(matches, []);
}

testFindsMatchesCaseInsensitivelyAcrossRows();
testReturnsEveryOccurrenceOnSameRow();
testIgnoresEmptyQueries();
