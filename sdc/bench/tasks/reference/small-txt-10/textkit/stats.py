from collections import Counter

from .tokenize import words


def word_freq(text):
    return Counter(words(text))


def top_n(text, n):
    """The n most frequent words as (word, count), most frequent first; ties alphabetical."""
    counts = word_freq(text)
    return sorted(counts.items(), key=lambda pair: (-pair[1], pair[0]))[:n]


def average_word_length(text):
    found = words(text)
    return sum(len(word) for word in found) / len(found)


def longest_word(text):
    best = ""

    for word in words(text):
        if len(word) > len(best):
            best = word

    return best
