from collections import Counter

from .tokenize import words


def word_freq(text):
    return Counter(words(text))


def top_n(text, n, stopwords=()):
    """The n most frequent words as (word, count), most frequent first; ties alphabetical."""
    skip = {word.lower() for word in stopwords}
    counts = {word: count for word, count in word_freq(text).items() if word not in skip}
    return sorted(counts.items(), key=lambda pair: (-pair[1], pair[0]))[:n]


def average_word_length(text):
    found = words(text)
    return sum(len(word) for word in found) / len(found)
